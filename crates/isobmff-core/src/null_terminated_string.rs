//! [`NullTerminatedString`], the null-terminated UTF-8 string of ISO/IEC 14496-12 §4.2

use alloc::string::{String, ToString as _};
use core::str;

use crate::error::{Error, byte_count};

/// Text field of a box, as the spec's `string` type carries it
///
/// The spec defines the type as a null-terminated string of UTF-8 characters,
/// and both halves of that are what this value promises: the text is UTF-8, and
/// it holds no NUL of its own, so writing one after it always terminates it
/// where it ends.
///
/// Reading is the lenient half. Files that leave the terminator off a field
/// running to the end of its box are common, so [`from_slice`](Self::from_slice)
/// accepts them; writing puts a terminator back either way.
///
/// # Examples
///
/// ```
/// use isobmff_core::NullTerminatedString;
///
/// // A field that ends at its terminator
/// let name = NullTerminatedString::from_slice(b"VideoHandler\0").unwrap();
/// assert_eq!(name.as_str(), "VideoHandler");
///
/// // A file that leaves the terminator off reads the same
/// assert_eq!(NullTerminatedString::from_slice(b"VideoHandler").unwrap(), name);
///
/// // Writing puts the terminator back, so the length counts it
/// assert_eq!(name.encoded_len(), 13);
///
/// let mut buffer = vec![0xff; 13];
/// assert!(name.encode(&mut buffer).unwrap().is_empty());
/// assert_eq!(buffer, b"VideoHandler\0");
///
/// // A string carrying a NUL of its own could not be read back whole
/// assert_eq!(NullTerminatedString::new(String::from("Video\0Handler")), None);
/// ```
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct NullTerminatedString(String);

impl NullTerminatedString {
    /// Creates the field from the text it carries
    ///
    /// Returns `None` when `value` holds a NUL, which the terminator written
    /// after it could not be told apart from.
    #[must_use]
    pub fn new(value: String) -> Option<Self> {
        if value.as_bytes().contains(&0) {
            return None;
        }

        Some(Self(value))
    }

    /// Returns the text the field carries, terminator excluded
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Reads the field that occupies the whole of `bytes`
    ///
    /// The text runs to the first NUL, or to the end of `bytes` where there is
    /// none. Bytes after a terminator are dropped: this reads a field that is
    /// the last of its box, so nothing that follows lays claim to them.
    ///
    /// # Errors
    ///
    /// * [`InvalidUtf8`](crate::ErrorKind::InvalidUtf8): the text is not UTF-8.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, Error> {
        let text = match bytes.iter().position(|byte| *byte == 0) {
            // Why not unwrap: the index `position` reports is within `bytes`, so
            // the range always slices, and a degenerate value stands in for the
            // panic the lints forbid.
            Some(terminator) => bytes.get(..terminator).unwrap_or(&[]),
            None => bytes,
        };

        Ok(Self(
            str::from_utf8(text)
                .map_err(|error| Error::invalid_utf8(error.valid_up_to()))?
                .to_string(),
        ))
    }

    /// Returns the length the field occupies, terminator included
    #[must_use]
    pub fn encoded_len(&self) -> u64 {
        // Why not unwrap: a usize above `u64::MAX` needs a 128-bit target to
        // exist, and saturating keeps the panic-free path.
        u64::try_from(self.0.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1)
    }

    /// Writes the field and its terminator into the front of `buffer` and
    /// returns what is left
    ///
    /// `buffer` is at least [`encoded_len`](Self::encoded_len) bytes long.
    ///
    /// # Errors
    ///
    /// * [`TruncatedBuffer`](crate::ErrorKind::TruncatedBuffer): `buffer` is shorter
    ///   than [`encoded_len`](Self::encoded_len).
    pub fn encode<'buffer>(&self, buffer: &'buffer mut [u8]) -> Result<&'buffer mut [u8], Error> {
        let needed = self.encoded_len();
        let too_short = Error::truncated_buffer(needed, byte_count(buffer.len()));

        let (whole, rest) = usize::try_from(needed)
            .ok()
            .and_then(|needed| buffer.split_at_mut_checked(needed))
            .ok_or(too_short)?;
        let (text, terminator) = whole.split_at_mut_checked(self.0.len()).ok_or(too_short)?;

        text.copy_from_slice(self.0.as_bytes());
        terminator.fill(0);

        Ok(rest)
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec;

    use super::NullTerminatedString;
    use crate::error::Error;

    #[test]
    fn a_field_holding_only_a_terminator_reads_as_the_empty_string() {
        assert_eq!(
            NullTerminatedString::from_slice(b"\0").unwrap(),
            NullTerminatedString(String::new())
        );
    }

    #[test]
    fn an_empty_field_reads_as_the_empty_string() {
        assert_eq!(
            NullTerminatedString::from_slice(b"").unwrap(),
            NullTerminatedString(String::new())
        );
    }

    #[test]
    fn bytes_after_the_terminator_are_dropped() {
        assert_eq!(
            NullTerminatedString::from_slice(b"name\0trailing").unwrap(),
            NullTerminatedString(String::from("name"))
        );
    }

    #[test]
    fn text_that_is_not_utf8_is_rejected() {
        assert_eq!(
            NullTerminatedString::from_slice(b"\xff\0"),
            Err(Error::invalid_utf8(0))
        );
    }

    #[test]
    fn multibyte_text_is_written_and_read_back_whole() {
        let field = NullTerminatedString::new(String::from("日本語")).unwrap();
        let mut buffer = vec![0xff; 10];

        field.encode(&mut buffer).unwrap();

        assert_eq!(field.encoded_len(), 10);
        assert_eq!(NullTerminatedString::from_slice(&buffer).unwrap(), field);
    }

    #[test]
    fn a_buffer_one_byte_short_of_the_terminator_is_refused() {
        let field = NullTerminatedString::new(String::from("name")).unwrap();

        assert_eq!(
            field.encode(&mut [0; 4]),
            Err(Error::truncated_buffer(5, 4))
        );
    }

    #[test]
    fn a_field_written_into_a_longer_buffer_leaves_the_rest_untouched() {
        let field = NullTerminatedString::new(String::from("ab")).unwrap();
        let mut buffer = [0xff; 6];

        let rest = field.encode(&mut buffer).unwrap();

        assert_eq!(rest, [0xff; 3]);
        assert_eq!(buffer, *b"ab\0\xff\xff\xff");
    }
}
