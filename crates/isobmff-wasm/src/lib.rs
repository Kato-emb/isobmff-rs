//! The `isobmff` crate as the browser reaches it, through `wasm-bindgen`
//!
//! A box, a state machine, [`BoxReader`], [`Boxes`], a [`Sample`] and the
//! enums holding boxes ([`SampleSizes`], [`ChunkOffsets`]) are classes named
//! after the library's types, and [`Mp4EpochSeconds`] a class of a static
//! method alone; every other value is a plain object, built by
//! `serde-wasm-bindgen`, whose TypeScript type is written by hand beside it
//! and which holds no wasm memory. Methods and the fields of a plain object
//! are named in camelCase. An enum carrying data is a union discriminated by
//! `type`, and an enum carrying none the camelCase name of its variant; a
//! value the specification states as one field is the value of that field,
//! and one it states as several an object of those fields, even where the
//! library holds them in one integer (`SampleFlags`); a four-character code
//! or a UUID is the string it displays as, `Option` is `undefined`, a `u64`
//! or an `i64` a `BigInt`, `Vec<u8>` a `Uint8Array`, `i32`s an `Int32Array`
//! whether taken or returned, an argument of `u32`s a `Uint32Array` and one
//! of `u64`s a `BigUint64Array`, another `Vec` or a tuple an array, and a
//! [`core::ops::Range`] an object of `start` and `end`.
//! [`Boxes`] is iterable, and a `poll*` method returns `undefined` once
//! nothing is ready.
//!
//! A box a state machine lends, or one a constructor or a `with*` method
//! makes, is a root of its own that the box's class holds; the class of a box
//! within it is a handle holding that root and the path to the box, followed
//! again on every call; a box the library lends no `_mut` path to is read
//! from a copy each call, and its class changes nothing in place.
//! `set<Child>(box)` puts a copy of `box` in place of the child, and a box
//! handed to a state machine is copied too. A `Sample` handed to
//! `handleSample` and the tracks handed to `MovieBox.new` or `newFragmented`
//! are taken by the call. Every other class is released by its user with
//! `free()` or `using`, but for the boxes `for…of` walks, which release what
//! they hold once used up or failed.
//!
//! A failure is thrown as an `Error` whose message is the one the library
//! states and whose `kind` names the layer that refused: `"box"`,
//! `"sequence"`, `"sample"` or `"structure"`, `"handle"` for a handle whose
//! path leads to no box, and `"binding"` for a value that did not convert to
//! or from a plain object or to the value the library takes.

extern crate alloc;

use serde::Serialize;
use serde_wasm_bindgen::Serializer;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::wasm_bindgen;

mod boxes;
mod framing;
mod sample;
mod sequence;
mod structure;

pub use boxes::{
    AnyBox, ChunkOffsets, EditBox, EditListBox, FileTypeBox, HandlerBox, JsSampleChunks, MediaBox,
    MediaHeaderBox, MediaInformationBox, MovieBox, MovieHeaderBox, Mp4EpochSeconds,
    SampleDescriptionBox, SampleSizeBox, SampleSizes, SampleTableBox, SampleToChunkBox,
    SyncSampleBox, TimeToSampleBox, TrackBox, TrackHeaderBox, container_boxes,
};
pub use framing::{Boxes, JsBoxType, JsIteratorStep};
pub use sample::{JsSampleProperties, Sample};
pub use sequence::{BoxReader, JsPolledEvent};
pub use structure::{
    FragmentedMuxFsm, JsSampleExtent, JsWantedInput, MovieDemuxFsm, MovieSampleExtentFsm,
    NonFragmentedMuxFsm,
};

/// [`core::ops::Range`] of `u64`
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct Range {
    /// [`core::ops::Range::start`]
    start: u64,
    /// [`core::ops::Range::end`]
    end: u64,
}

#[wasm_bindgen(typescript_custom_section)]
const RANGE: &str = r#"
/** `core::ops::Range` of `u64` */
export interface Range {
    start: bigint;
    end: bigint;
}
"#;

impl From<core::ops::Range<u64>> for Range {
    fn from(range: core::ops::Range<u64>) -> Self {
        Self {
            start: range.start,
            end: range.end,
        }
    }
}

/// A refusal of the library, of a handle or of a conversion to or from a plain object, thrown as an `Error` carrying the `kind` of the layer that refused
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

impl From<serde_wasm_bindgen::Error> for Failure {
    fn from(error: serde_wasm_bindgen::Error) -> Self {
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

/// Returns `value` as the plain object `T` names, its `u64`s and `i64`s as `BigInt`s
///
/// # Errors
///
/// `"binding"`: `value` did not convert to a plain object.
fn to_plain<T: JsCast>(value: &impl Serialize) -> Result<T, Failure> {
    Ok(value
        .serialize(&Serializer::new().serialize_large_number_types_as_bigints(true))?
        .unchecked_into())
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
