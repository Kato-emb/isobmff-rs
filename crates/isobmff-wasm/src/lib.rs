//! The `isobmff` crate as the browser reaches it, through `wasm-bindgen`
//!
//! Each export takes the file as a `Blob` and reads it through
//! `FileReaderSync`, and [`remux`] writes its output through a
//! `FileSystemSyncAccessHandle`; both exist only in a worker, so every export
//! is called from a worker. A failure reaches JavaScript as an `Error` whose
//! message is the one the library states, or the exception the browser threw.

use core::{error, fmt};
use std::io::{self, BufWriter};

use isobmff::Mp4EpochSeconds;
use isobmff::structure;
use wasm_bindgen::JsError;
use wasm_bindgen::prelude::wasm_bindgen;
use web_sys::{Blob, FileSystemSyncAccessHandle};

use crate::sink::OpfsSink;
use crate::source::{BlobSource, js_integer};

mod demux;
mod dump;
mod prototype_js_binding;
mod remux;
mod sink;
mod source;

pub use demux::{Demux, SampleRecord, TrackRecord};
pub use dump::BoxRecord;
pub use remux::Output;

/// Reason an export fails: the browser failed it, the input is one an export does not take, or the library refused it
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

/// Returns the boxes `blob` is formed as, each container followed by the boxes it holds
///
/// # Errors
///
/// The failure of reading `blob`, or the reason the library refuses the boxes
/// it reads: a box or a header that runs past the end of the file, a declared
/// size smaller than its header, a box that ends past `u64::MAX`, or a
/// container whose payload does not divide into boxes.
#[wasm_bindgen]
pub fn dump_boxes(blob: Blob) -> Result<Vec<BoxRecord>, JsError> {
    Ok(dump::dump(BlobSource::new(blob)?)?)
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
