//! The `isobmff` crate as the browser reaches it, through `wasm-bindgen`
//!
//! A value of the standard library comes back as a plain JavaScript value,
//! and a value of the library as a class: `None` is `undefined`, a `u64` or
//! an `i64` a `BigInt`, a `Vec<u8>` a `Uint8Array`, a `Vec` of another type
//! and a tuple an array, and a [`core::ops::Range`] an object of `start` and
//! `end`.
//!
//! A class named after a library type wraps that type, its methods are the
//! library's methods of the same names, and a public field of the type is a
//! property of the same name. A method returning a reference returns a copy.
//! Each variant of an enum carrying data is read through a method named after
//! it, which returns the variant's data, or `undefined` for another variant,
//! and `true` or `false` for a variant carrying none. What the library
//! displays is the `toString` of the class, a constant is a function of the
//! same name, and the `next` of an iterator returns the item, or `undefined`
//! once the items are used up.
//!
//! [`remux()`] is not of the library: it takes the file as a `Blob` and reads
//! it through `FileReaderSync`, and writes its output through a
//! `FileSystemSyncAccessHandle`; both exist only in a worker, so it is called
//! from a worker. A failure reaches JavaScript as an `Error` whose message is
//! the one the library states, or the exception the browser threw.

extern crate alloc;

use std::io::{self, BufWriter};

use isobmff::Mp4EpochSeconds;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsError;
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
    HandlerBox, HeaderDuration, MediaBox, MediaHeaderBox, MediaInformationBox, MovieBox,
    SampleDescriptionBox, SampleFlags, SampleTableBox, TrackBox, TrackHeaderBox, container_boxes,
};
pub use framing::{AnyBox, BoxHeader, BoxSize, BoxType, Boxes, FourCc, RawBox};
pub use remux::Output;
pub use sample::{SampleExtent, SampleProperties};
pub use sequence::{BoxEvent, BoxReader};
pub use structure::MovieSampleExtentFsm;

#[wasm_bindgen]
extern "C" {
    /// [`core::ops::Range`] of `u64` as a plain object of `start` and `end`
    #[wasm_bindgen(typescript_type = "{ start: bigint; end: bigint }")]
    pub type Range;

    #[wasm_bindgen(method, setter)]
    fn set_start(this: &Range, start: u64);

    #[wasm_bindgen(method, setter)]
    fn set_end(this: &Range, end: u64);
}

impl From<core::ops::Range<u64>> for Range {
    fn from(range: core::ops::Range<u64>) -> Self {
        let object = js_sys::Object::new().unchecked_into::<Self>();
        object.set_start(range.start);
        object.set_end(range.end);
        object
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
