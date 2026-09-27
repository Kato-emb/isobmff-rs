//! [`BlobSource`], a `Blob` read as a [`Read`]

use std::io::{self, Read};

use js_sys::Uint8Array;
use wasm_bindgen::JsValue;
use web_sys::{Blob, FileReaderSync};

/// The largest integer a JavaScript number holds exactly, `Number.MAX_SAFE_INTEGER`
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// A `Blob` read from its start, one `Blob.slice` per read
#[derive(Debug)]
pub(crate) struct BlobSource {
    blob: Blob,
    reader: FileReaderSync,
    position: u64,
}

impl BlobSource {
    /// Returns a source reading `blob` from its start
    ///
    /// # Errors
    ///
    /// `FileReaderSync` is not available, which it is not outside a worker.
    pub(crate) fn new(blob: Blob) -> io::Result<Self> {
        Ok(Self {
            blob,
            reader: FileReaderSync::new().map_err(js_failure)?,
            position: 0,
        })
    }
}

impl Read for BlobSource {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let end = self
            .position
            .saturating_add(u64::try_from(buffer.len()).unwrap_or(u64::MAX))
            .min(MAX_SAFE_INTEGER);
        let slice = self
            .blob
            .slice_with_f64_and_f64(js_number(self.position), js_number(end))
            .map_err(js_failure)?;
        let array_buffer = self
            .reader
            .read_as_array_buffer(&slice)
            .map_err(js_failure)?;
        let bytes = Uint8Array::new(&array_buffer);
        let read = usize::try_from(bytes.length()).map_err(io::Error::other)?;
        bytes.copy_to(
            buffer
                .get_mut(..read)
                .ok_or_else(|| io::Error::other("a Blob slice came back longer than asked"))?,
        );
        self.position = self
            .position
            .saturating_add(u64::try_from(read).map_err(io::Error::other)?);
        Ok(read)
    }
}

/// Returns `value`, at most [`MAX_SAFE_INTEGER`], as the JavaScript number equal to it
#[expect(
    clippy::cast_precision_loss,
    reason = "every value passed is at most MAX_SAFE_INTEGER, which an f64 holds exactly"
)]
const fn js_number(value: u64) -> f64 {
    value as f64
}

/// Returns the exception a browser API threw as an I/O failure
fn js_failure(exception: JsValue) -> io::Error {
    io::Error::other(format!("{exception:?}"))
}
