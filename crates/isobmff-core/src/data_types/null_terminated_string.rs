//! [`NullTerminatedString`], the null-terminated string type of ISO/IEC 14496-12 §4.2

use alloc::string::String;
use alloc::vec::Vec;
use core::{fmt, str};

use crate::error::Error;

/// Text field of a box, as the spec's `string` type carries it
///
/// The spec defines the type as a null-terminated string of UTF-8 characters.
/// The value holds the bytes before the terminator, which hold no NUL of their
/// own, so writing one after them always terminates them where they end.
///
/// Reading is the lenient half. Files that leave the terminator off a field
/// running to the end of its box are common, and so are files whose text is in
/// an encoding other than UTF-8, so [`from_slice`](Self::from_slice) accepts
/// both: [`as_str`](Self::as_str) returns the text where the bytes are UTF-8,
/// and [`as_bytes`](Self::as_bytes) returns them whatever they are. Writing
/// puts the bytes back as they were read, and a terminator after them.
///
/// # Examples
///
/// ```
/// use isobmff_core::NullTerminatedString;
///
/// // A field that ends at its terminator
/// let name = NullTerminatedString::from_slice(b"VideoHandler\0");
/// assert_eq!(name.as_str(), Some("VideoHandler"));
///
/// // A file that leaves the terminator off reads the same
/// assert_eq!(NullTerminatedString::from_slice(b"VideoHandler"), name);
///
/// // Text that is not UTF-8 is kept as its bytes
/// let mac_roman = NullTerminatedString::from_slice(b"Caf\x8e\0");
/// assert_eq!(mac_roman.as_str(), None);
/// assert_eq!(mac_roman.as_bytes(), b"Caf\x8e");
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
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct NullTerminatedString(Vec<u8>);

impl NullTerminatedString {
    /// Creates the field from the text it carries
    ///
    /// Returns `None` when `value` holds a NUL, which the terminator written
    /// after it could not be told apart from. The field that carries no text is
    /// [`default`](Self::default): the empty string, one NUL byte on the wire.
    #[must_use]
    pub fn new(value: String) -> Option<Self> {
        if value.as_bytes().contains(&0) {
            return None;
        }

        Some(Self(value.into_bytes()))
    }

    /// Returns the text the field carries, terminator excluded, or `None` where its bytes are not UTF-8
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        str::from_utf8(&self.0).ok()
    }

    /// Returns the bytes the field carries, terminator excluded
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Reads the field that occupies the whole of `bytes`
    ///
    /// The field runs to the first NUL, or to the end of `bytes` where there is
    /// none. Bytes after a terminator are dropped: this reads a field that is
    /// the last of its box, so nothing that follows lays claim to them.
    #[must_use]
    pub fn from_slice(bytes: &[u8]) -> Self {
        let text = match bytes.iter().position(|byte| *byte == 0) {
            // Why not unwrap: the index `position` reports is within `bytes`, so
            // the range always slices, and a degenerate value stands in for the
            // panic the lints forbid.
            Some(terminator) => bytes.get(..terminator).unwrap_or(&[]),
            None => bytes,
        };

        Self(text.to_vec())
    }

    /// Returns the length the field occupies, terminator included
    #[must_use]
    pub fn encoded_len(&self) -> u64 {
        (self.0.len() as u64).saturating_add(1)
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
        let too_short = Error::truncated_buffer(needed, buffer.len() as u64);

        let (whole, rest) = usize::try_from(needed)
            .ok()
            .and_then(|needed| buffer.split_at_mut_checked(needed))
            .ok_or(too_short)?;
        let (text, terminator) = whole.split_at_mut_checked(self.0.len()).ok_or(too_short)?;

        text.copy_from_slice(&self.0);
        terminator.fill(0);

        Ok(rest)
    }
}

impl fmt::Debug for NullTerminatedString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "NullTerminatedString(\"{}\")",
            self.0.escape_ascii()
        )
    }
}

#[cfg(test)]
mod tests {
    use alloc::format;
    use alloc::string::String;
    use alloc::vec;
    use alloc::vec::Vec;

    use super::NullTerminatedString;
    use crate::error::Error;

    #[test]
    fn a_field_holding_only_a_terminator_reads_as_the_empty_string() {
        assert_eq!(
            NullTerminatedString::from_slice(b"\0"),
            NullTerminatedString(Vec::new())
        );
    }

    #[test]
    fn an_empty_field_reads_as_the_empty_string() {
        assert_eq!(
            NullTerminatedString::from_slice(b""),
            NullTerminatedString(Vec::new())
        );
    }

    #[test]
    fn bytes_after_the_terminator_are_dropped() {
        assert_eq!(
            NullTerminatedString::from_slice(b"name\0trailing"),
            NullTerminatedString(b"name".to_vec())
        );
    }

    #[test]
    fn text_that_is_not_utf8_is_kept_as_its_bytes_and_written_back_as_them() {
        let field = NullTerminatedString::from_slice(b"\xff\0");
        let mut buffer = vec![0xaa; 2];

        field.encode(&mut buffer).unwrap();

        assert_eq!(field, NullTerminatedString(vec![0xff]));
        assert_eq!(buffer, b"\xff\0");
    }

    #[test]
    fn multibyte_text_is_written_and_read_back_whole() {
        let field = NullTerminatedString::new(String::from("日本語")).unwrap();
        let mut buffer = vec![0xff; 10];

        field.encode(&mut buffer).unwrap();

        assert_eq!(field.encoded_len(), 10);
        assert_eq!(NullTerminatedString::from_slice(&buffer), field);
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

    #[test]
    fn debug_shows_the_bytes_escaped_as_ascii() {
        let field = NullTerminatedString::from_slice(b"Caf\x8e");

        assert_eq!(format!("{field:?}"), r#"NullTerminatedString("Caf\x8e")"#);
    }
}
