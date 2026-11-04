//! [`ByteRange`], a run of bytes of a file named by where it starts

/// Contiguous run of bytes of a file: a file offset and, when it is known, a length
///
/// The offset counts from the first byte of the file. A length of `None`
/// leaves the run open: it goes on for as many bytes as there are.
///
/// # Examples
///
/// ```
/// use isobmff::ByteRange;
///
/// // The eight bytes from file offset 32
/// let header = ByteRange::new(32, Some(8));
/// assert_eq!(header.offset(), 32);
/// assert_eq!(header.length(), Some(8));
///
/// // Everything from file offset 40 on
/// assert_eq!(ByteRange::new(40, None).length(), None);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ByteRange {
    offset: u64,
    length: Option<u64>,
}

impl ByteRange {
    /// Creates the run of `length` bytes from file offset `offset`, or of every byte from it on
    #[must_use]
    pub const fn new(offset: u64, length: Option<u64>) -> Self {
        Self { offset, length }
    }

    /// Returns the file offset of the first byte of the run
    #[must_use]
    pub const fn offset(self) -> u64 {
        self.offset
    }

    /// Returns the length of the run, or `None` when it runs on for as many bytes as there are
    #[must_use]
    pub const fn length(self) -> Option<u64> {
        self.length
    }
}
