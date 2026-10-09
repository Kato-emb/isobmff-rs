//! [`BlobSource`], a `Blob` read as a [`Read`] and [`Seek`]

use std::io::{self, Read, Seek, SeekFrom};

use js_sys::{BigInt, Uint8Array};
use wasm_bindgen::JsValue;
use web_sys::{Blob, FileReaderSync};

/// The largest integer a JavaScript number holds exactly, `Number.MAX_SAFE_INTEGER`
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// A `Blob` read from where it was last sought, or from its start, one `Blob.slice` per read
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
        let start = self.position.min(MAX_SAFE_INTEGER);
        let end = start
            .saturating_add(u64::try_from(buffer.len()).unwrap_or(u64::MAX))
            .min(MAX_SAFE_INTEGER);
        let slice = self
            .blob
            .slice_with_f64_and_f64(js_number(start), js_number(end))
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

impl Seek for BlobSource {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let (base, offset) = match position {
            SeekFrom::Start(offset) => {
                self.position = offset;
                return Ok(offset);
            }
            SeekFrom::End(offset) => (js_integer(self.blob.size())?, offset),
            SeekFrom::Current(offset) => (self.position, offset),
        };
        self.position = base.checked_add_signed(offset).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "a seek before the start of the Blob or past u64::MAX",
            )
        })?;
        Ok(self.position)
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

/// Returns the integer the JavaScript number `value` holds
///
/// # Errors
///
/// `value` is not a whole number, or not one a `u64` holds.
pub(crate) fn js_integer(value: f64) -> io::Result<u64> {
    let integer =
        BigInt::new(&JsValue::from_f64(value)).map_err(|exception| js_failure(exception.into()))?;
    u64::try_from(integer).map_err(|integer| io::Error::other(format!("{integer:?} is not a u64")))
}

/// Returns the exception a browser API threw as an I/O failure
pub(crate) fn js_failure(exception: JsValue) -> io::Error {
    io::Error::other(format!("{exception:?}"))
}
