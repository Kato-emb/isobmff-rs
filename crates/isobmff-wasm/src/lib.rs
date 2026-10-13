//! The `isobmff` crate as the browser reaches it, through `wasm-bindgen`
//!
//! A class named after a library type wraps that type, and its methods are
//! the library's methods of the same names. A value of another library type
//! comes back wrapped, a tuple comes back as an array, and a
//! [`core::ops::Range`] as a [`Range`]. Each variant of an enum carrying data
//! is read through a method named after it, which returns the variant's data,
//! or `undefined` for another variant, and `true` or `false` for a variant
//! carrying none. What the library displays is the `toString` of the class,
//! a constant is a function of the same name, and the `next` of an iterator
//! returns the item, or `undefined` once the items are used up.
//!
//! [`demux()`] and [`remux()`] are not of the library: they take the file as
//! a `Blob` and read it through `FileReaderSync`, and [`remux()`] writes its
//! output through a `FileSystemSyncAccessHandle`; both exist only in a
//! worker, so those two are called from a worker. A failure reaches
//! JavaScript as an `Error` whose message is the one the library states, or
//! the exception the browser threw.

extern crate alloc;

use core::{error, fmt};
use std::io::{self, BufWriter};

use isobmff::Mp4EpochSeconds;
use isobmff::structure;
use wasm_bindgen::JsError;
use wasm_bindgen::prelude::wasm_bindgen;
use web_sys::{Blob, FileSystemSyncAccessHandle};

use crate::sink::OpfsSink;
use crate::source::{BlobSource, js_integer};

mod boxes;
mod demux;
mod framing;
mod remux;
mod sequence;
mod sink;
mod source;

pub use boxes::container_boxes;
pub use demux::{Demux, SampleRecord, TrackRecord};
pub use framing::{BoxHeader, BoxSize, BoxType, Boxes, RawBox};
pub use remux::Output;
pub use sequence::{BoxEvent, BoxReader};

/// [`core::ops::Range`] of `u64`, the extent of a file the library hands over
#[wasm_bindgen]
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct Range {
    /// [`core::ops::Range::start`]
    pub start: u64,
    /// [`core::ops::Range::end`]
    pub end: u64,
}

/// Reason [`demux()`] or [`remux()`] fails: the browser failed it, the input is one it does not take, or the library refused it
#[derive(Debug)]
pub(crate) enum Error {
    /// The `Blob` did not read, the output did not write, the clock did not tell the time, or the input is one an export does not take
    Io(io::Error),
    /// The library refused what it read or what it was to write
    Structure(structure::Error),
}

impl From<io::Error> for Error {
    fn from(failure: io::Error) -> Self {
        Self::Io(failure)
    }
}

impl From<structure::Error> for Error {
    fn from(failure: structure::Error) -> Self {
        Self::Structure(failure)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(failure) => write!(formatter, "{failure}"),
            Self::Structure(failure) => write!(formatter, "{failure}"),
        }
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Io(failure) => Some(failure),
            Self::Structure(failure) => Some(failure),
        }
    }
}

/// Returns the tracks of the movie `blob` carries and the samples its tables declare
///
/// # Errors
///
/// The failure of reading `blob`, the reason the library refuses the boxes it
/// reads or the tables the `moov` and each `moof` declare, or a file that
/// carries no `moov`, or none ahead of its first `moof` as a media segment
/// delivered apart from its movie does.
#[wasm_bindgen]
pub fn demux(blob: Blob) -> Result<Demux, JsError> {
    Ok(demux::demux(BlobSource::new(blob)?)?)
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
