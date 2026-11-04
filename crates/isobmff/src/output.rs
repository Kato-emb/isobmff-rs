//! [`Output`], the bytes a writer returns and the file offset they go to

use alloc::vec::Vec;

/// Bytes a writer returns, and the file offset they go to
///
/// The offset counts from the first byte of the file the writer writes. A
/// writer returns its output in the order the caller is to write it. The
/// caller keeps where the file ends, and writes each output there by its
/// offset:
///
/// * an offset at the end of the file extends the file with the bytes;
/// * an offset before the end, the bytes ending no later than it, writes the
///   bytes over those the file holds there;
/// * any other offset is a failure of the writer.
///
/// A caller that cannot seek, such as one writing to a pipe, takes outputs
/// whose offset is the end of the file, and stops at the first that is not.
///
/// # Examples
///
/// ```
/// use isobmff::boxes::FileTypeBox;
/// use isobmff::framing::BoxWriter;
/// use isobmff::FourCc;
///
/// let mut ftyp = FileTypeBox::default();
/// ftyp.major_brand = FourCc::new(*b"isom");
/// let mut boxes = BoxWriter::new();
/// let outputs = [boxes.write_box(&ftyp)?, boxes.write_reserved(64)?];
///
/// // A sink that cannot seek writes each output at the end, and stops at any other
/// let mut pipe: Vec<u8> = Vec::new();
/// for output in outputs {
///     if output.offset() != u64::try_from(pipe.len())? {
///         return Err("the output goes elsewhere than the end".into());
///     }
///     pipe.extend_from_slice(output.bytes());
/// }
/// assert_eq!(pipe.get(4..8), Some(b"ftyp".as_slice()));
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Output {
    offset: u64,
    bytes: Vec<u8>,
}

impl Output {
    /// Creates the output of `bytes` that go to file offset `offset`
    ///
    /// A writer outside this crate returns its output as this crate's
    /// writers do, under the same contract, so that one loop writes the
    /// output of either.
    #[must_use]
    pub const fn new(offset: u64, bytes: Vec<u8>) -> Self {
        Self { offset, bytes }
    }

    /// Returns the file offset the bytes go to
    #[must_use]
    pub const fn offset(&self) -> u64 {
        self.offset
    }

    /// Returns the bytes
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the bytes, giving up the offset
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}
