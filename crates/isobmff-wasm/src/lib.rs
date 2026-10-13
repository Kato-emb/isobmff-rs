//! The `isobmff` crate as the browser reaches it, through `wasm-bindgen`
//!
//! A box, a state machine, [`BoxReader`] and [`Boxes`] are classes named
//! after the library's types; every other value is a plain object, whose
//! TypeScript type is generated and which holds no wasm memory. Methods and
//! the fields of a plain object are named in camelCase. An enum carrying data
//! is a union discriminated by `type`, and an enum carrying none the camelCase
//! name of its variant; a newtype is its inner value, `Option` is
//! `undefined`, a `u64` or an `i64` a `BigInt`, `Vec<u8>` a `Uint8Array`,
//! another `Vec` or a tuple an array, and a [`core::ops::Range`] an object of
//! `start` and `end`. [`Boxes`] is iterable, and a `poll*` method returns
//! `undefined` once nothing is ready.
//!
//! A box a state machine lends is copied once into a root that the box's
//! class holds; the class of a box within it is a handle holding that root
//! and the path to the box, followed again on every call. Each class is
//! released by its user with `free()` or `using`, but for the boxes `for…of`
//! walks, which release what they hold once used up or failed.
//!
//! A failure is thrown as an `Error` whose message is the one the library
//! states and whose `kind` names the layer that refused: `"box"`,
//! `"sequence"`, `"sample"` or `"structure"`, `"handle"` for a handle whose
//! path leads to no box, and `"binding"` for a value that did not convert to
//! a plain object.
//!
//! [`remux()`] is not of the library: it takes the file as a `Blob` and reads
//! it through `FileReaderSync`, and writes its output through a
//! `FileSystemSyncAccessHandle`; both exist only in a worker, so it is called
//! from a worker. Its failure is an `Error` whose message is the one the
//! library states, or the exception the browser threw.

extern crate alloc;

use std::io::{self, BufWriter};

use isobmff::Mp4EpochSeconds;
use serde::Serialize;
use tsify::Tsify;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsError;
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::wasm_bindgen;
use web_sys::{Blob, FileSystemSyncAccessHandle};

use crate::sink::OpfsSink;
use crate::source::{BlobSource, js_integer};

mod boxes;
mod framing;
mod remux;
mod sample;
mod sequence;
mod sink;
mod source;
mod structure;

pub use boxes::{
    AnyBox, DegradationPriorityEntry, HandlerBox, HeaderDuration, IsLeading, MediaBox,
    MediaHeaderBox, MediaInformationBox, MovieBox, PaddingBitsEntry, SampleDependencyTypeEntry,
    SampleDependsOn, SampleDescriptionBox, SampleFlags, SampleHasRedundancy, SampleIsDependedOn,
    SampleTableBox, TrackBox, TrackHeaderBox, container_boxes,
};
pub use framing::{BoxHeader, BoxSize, BoxType, Boxes, FourCc, IteratorStep, RawBox};
pub use remux::Output;
pub use sample::{SampleExtent, SampleProperties};
pub use sequence::{BoxEvent, BoxReader};
pub use structure::{MovieSampleExtentFsm, WantedInput};

/// [`core::ops::Range`] of `u64`
#[derive(Debug, Clone, Copy, Serialize, Tsify)]
#[tsify(large_number_types_as_bigints)]
#[non_exhaustive]
pub struct Range {
    /// [`core::ops::Range::start`]
    pub start: u64,
    /// [`core::ops::Range::end`]
    pub end: u64,
}

impl From<core::ops::Range<u64>> for Range {
    fn from(range: core::ops::Range<u64>) -> Self {
        Self {
            start: range.start,
            end: range.end,
        }
    }
}

/// A refusal of the library, of a handle or of a conversion to a plain object, thrown as an `Error` carrying the `kind` of the layer that refused
#[derive(Debug)]
pub struct Failure {
    /// The layer that refused
    kind: &'static str,
    /// What the library states of the refusal
    message: String,
}

impl Failure {
    /// Returns the failure of a handle whose path leads to no box
    pub(crate) fn handle() -> Self {
        Self {
            kind: "handle",
            message: "the box this handle names is no longer there".to_owned(),
        }
    }
}

impl From<tsify::Error> for Failure {
    fn from(error: tsify::Error) -> Self {
        Self {
            kind: "binding",
            message: error.to_string(),
        }
    }
}

impl From<isobmff::Error> for Failure {
    fn from(error: isobmff::Error) -> Self {
        Self {
            kind: "box",
            message: error.to_string(),
        }
    }
}

impl From<isobmff::sequence::Error> for Failure {
    fn from(error: isobmff::sequence::Error) -> Self {
        Self {
            kind: sequence_kind(&error),
            message: error.to_string(),
        }
    }
}

impl From<isobmff::sample::Error> for Failure {
    fn from(error: isobmff::sample::Error) -> Self {
        Self {
            kind: sample_kind(&error),
            message: error.to_string(),
        }
    }
}

impl From<isobmff::structure::Error> for Failure {
    fn from(error: isobmff::structure::Error) -> Self {
        let kind = match &error {
            isobmff::structure::Error::Sequence { error, .. } => sequence_kind(error),
            isobmff::structure::Error::Sample { error, .. } => sample_kind(error),
            isobmff::structure::Error::Box { .. } => "box",
            _ => "structure",
        };
        Self {
            kind,
            message: error.to_string(),
        }
    }
}

/// Returns the layer a refusal of the sequence layer came from: the box it holds, or the sequence layer itself
fn sequence_kind(error: &isobmff::sequence::Error) -> &'static str {
    match error {
        isobmff::sequence::Error::Box { .. } => "box",
        _ => "sequence",
    }
}

/// Returns the layer a refusal of the sample layer came from: the box it holds, or the sample layer itself
fn sample_kind(error: &isobmff::sample::Error) -> &'static str {
    match error {
        isobmff::sample::Error::Box { .. } => "box",
        _ => "sample",
    }
}

#[wasm_bindgen]
extern "C" {
    /// An `Error` given the `kind` of the layer that refused
    #[wasm_bindgen(extends = js_sys::Error)]
    type ErrorOfLayer;

    #[wasm_bindgen(method, setter)]
    fn set_kind(this: &ErrorOfLayer, kind: &str);
}

impl From<Failure> for JsValue {
    fn from(failure: Failure) -> Self {
        let error = js_sys::Error::new(&failure.message).unchecked_into::<ErrorOfLayer>();
        error.set_kind(failure.kind);
        error.into()
    }
}

/// Writes the movie file `blob` carries, fragmented or not, through `handle` in the layout `output` names
///
/// The file is written from where the cursor of `handle` stands. As
/// [`Fragmented`](Output::Fragmented), each track keeps its boxes but for its
/// sample tables, its samples lie in the fragments alone, written in decode
/// time across the tracks, and a fragment opens at every sync sample of a
/// track that carries a sync sample table. As
/// [`NonFragmented`](Output::NonFragmented), the movie header and the tracks
/// are kept with their modification times set to now and their sample tables
/// and durations made anew from the samples, every other box of the movie is
/// dropped, and a chunk opens wherever the samples pass to another track or
/// another sample description.
///
/// # Errors
///
/// The failure of reading `blob` or of writing through `handle`; the reason
/// the library refuses what it reads or what it is to write, a file that
/// carries no `moov` or none ahead of its first `moof` among them; a movie
/// declaring no track or two tracks of the same `track_ID`; more fragments
/// than a sequence number counts; or a clock past what a header states.
#[wasm_bindgen]
pub fn remux(
    blob: Blob,
    output: Output,
    handle: FileSystemSyncAccessHandle,
) -> Result<(), JsError> {
    let source = BlobSource::new(blob)?;
    let sink = BufWriter::new(OpfsSink(handle));
    match output {
        Output::Fragmented => remux::remux_to_fragmented(source, sink)?,
        Output::NonFragmented => {
            let now = Mp4EpochSeconds::from_unix_seconds(js_integer(js_sys::Date::now())? / 1000)
                .ok_or_else(|| io::Error::other("the clock is past what a header states"))?;
            remux::remux_to_non_fragmented(source, sink, now)?;
        }
    }
    Ok(())
}
