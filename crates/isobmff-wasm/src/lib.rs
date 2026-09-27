//! The `isobmff` crate as the browser reaches it, through `wasm-bindgen`
//!
//! Each export takes the file as a `Blob` and reads it through
//! `FileReaderSync`, which exists only in a worker, so every export is called
//! from a worker. A failure reaches JavaScript as an `Error` whose message is
//! the one the library states, or the exception the browser threw.

use wasm_bindgen::JsError;
use wasm_bindgen::prelude::wasm_bindgen;
use web_sys::Blob;

use crate::source::BlobSource;

mod dump;
mod source;

pub use dump::BoxRecord;

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
