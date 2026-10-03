//! The `isobmff` crate as the browser reaches it, through `wasm-bindgen`
//!
//! Each export takes the file as a `Blob` and reads it through
//! `FileReaderSync`, which exists only in a worker, so every export is called
//! from a worker. A failure reaches JavaScript as an `Error` whose message is
//! the one the library states, or the exception the browser threw.

use core::{error, fmt};
use std::io;

use isobmff::structure;
use wasm_bindgen::JsError;
use wasm_bindgen::prelude::wasm_bindgen;
use web_sys::Blob;

use crate::source::BlobSource;

mod demux;
mod dump;
mod source;

pub use demux::{Demux, SampleRecord, TrackRecord};
pub use dump::BoxRecord;

/// Reason an export fails: the `Blob` did not read, or the library refused what it read
#[derive(Debug)]
pub(crate) enum Error {
    /// The `Blob` did not read, or the file is one an export does not take
    Io(io::Error),
    /// The library refused what it read
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
