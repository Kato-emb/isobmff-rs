//! [`Utf8String`], a null-terminated UTF-8 string

use alloc::string::String;
use alloc::vec::Vec;
use core::{fmt, str};

use crate::error::Error;

/// Field of the basic type `utf8string`, ISO/IEC 14496-12 §4.2.1
///
/// The specification defines the type as a UTF-8 string ended by a NUL, which
/// shall be present. The value holds the bytes before the terminator, which
/// hold no NUL of their own, so writing one after them always terminates them
/// where they end. The `utfstring` type, which may be UTF-16 ended by two NUL
/// bytes, is not this one.
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
/// use isobmff_core::Utf8String;
///
/// // A field that ends at its terminator
/// let name = Utf8String::from_slice(b"VideoHandler\0");
/// assert_eq!(name.as_str(), Some("VideoHandler"));
///
/// // A file that leaves the terminator off reads the same
/// assert_eq!(Utf8String::from_slice(b"VideoHandler"), name);
///
/// // Text that is not UTF-8 is kept as its bytes
/// let mac_roman = Utf8String::from_slice(b"Gr\x9fn\0");
/// assert_eq!(mac_roman.as_str(), None);
/// assert_eq!(mac_roman.as_bytes(), b"Gr\x9fn");
///
/// // Writing puts the terminator back, so the length counts it
/// assert_eq!(name.encoded_len(), 13);
///
/// let mut buffer = vec![0xff; 13];
/// assert!(name.encode(&mut buffer).unwrap().is_empty());
/// assert_eq!(buffer, b"VideoHandler\0");
///
/// // A string carrying a NUL of its own could not be read back whole
/// assert_eq!(Utf8String::new(String::from("Video\0Handler")), None);
/// ```
#[doc(alias = "utf8string", alias = "NullTerminatedString")]
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct Utf8String(Vec<u8>);

impl Utf8String {
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
        let text = bytes.split(|byte| *byte == 0).next().unwrap_or_default();

        Self(text.to_vec())
    }

    /// Returns the length the field occupies, terminator included
    #[must_use]
    pub fn encoded_len(&self) -> u64 {
        u64::try_from(self.0.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1)
    }

    /// Writes the field and its terminator into the front of `buffer` and returns what is left
    ///
    /// # Errors
    ///
    /// * [`TruncatedBuffer`](crate::ErrorKind::TruncatedBuffer): `buffer` is shorter
    ///   than [`encoded_len`](Self::encoded_len).
    pub fn encode<'buffer>(&self, buffer: &'buffer mut [u8]) -> Result<&'buffer mut [u8], Error> {
        let available_bytes = buffer.len();
        let too_short = || {
            Error::truncated_buffer(
                self.encoded_len(),
                u64::try_from(available_bytes).unwrap_or(u64::MAX),
            )
        };

        let (text, rest) = buffer
            .split_at_mut_checked(self.0.len())
            .ok_or_else(too_short)?;
        let (terminator, rest) = rest.split_first_mut().ok_or_else(too_short)?;

        text.copy_from_slice(&self.0);
        *terminator = 0;

        Ok(rest)
    }
}

impl fmt::Debug for Utf8String {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Utf8String(\"{}\")", self.0.escape_ascii())
    }
}

#[cfg(test)]
mod tests {
    use alloc::format;
    use alloc::string::String;
    use alloc::vec;
    use alloc::vec::Vec;

    use super::Utf8String;
    use crate::error::Error;

    #[test]
    fn a_field_holding_only_a_terminator_reads_as_the_empty_string() {
        assert_eq!(Utf8String::from_slice(b"\0"), Utf8String(Vec::new()));
    }

    #[test]
    fn an_empty_field_reads_as_the_empty_string() {
        assert_eq!(Utf8String::from_slice(b""), Utf8String(Vec::new()));
    }

    #[test]
    fn bytes_after_the_terminator_are_dropped() {
        assert_eq!(
            Utf8String::from_slice(b"name\0trailing"),
            Utf8String(b"name".to_vec())
        );
    }

    #[test]
    fn text_that_is_not_utf8_is_kept_as_its_bytes_and_written_back_as_them() {
        let field = Utf8String::from_slice(b"\xff\0");
        let mut buffer = vec![0xaa; 2];

        field.encode(&mut buffer).unwrap();

        assert_eq!(field, Utf8String(vec![0xff]));
        assert_eq!(buffer, b"\xff\0");
    }

    #[test]
    fn multibyte_text_is_written_as_its_utf8_bytes_and_a_terminator() {
        let field = Utf8String::new(String::from("日本語")).unwrap();
        let mut buffer = vec![0xff; 10];

        field.encode(&mut buffer).unwrap();

        assert_eq!(field.encoded_len(), 10);
        assert_eq!(buffer, b"\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e\0".as_slice());
    }

    #[test]
    fn a_buffer_one_byte_short_of_the_terminator_is_refused() {
        let field = Utf8String::new(String::from("name")).unwrap();

        assert_eq!(
            field.encode(&mut [0; 4]),
            Err(Error::truncated_buffer(5, 4))
        );
    }

    #[test]
    fn a_field_written_into_a_longer_buffer_leaves_the_rest_untouched() {
        let field = Utf8String::new(String::from("ab")).unwrap();
        let mut buffer = [0xff; 6];

        let rest = field.encode(&mut buffer).unwrap();

        assert_eq!(rest, [0xff; 3]);
        assert_eq!(buffer, *b"ab\0\xff\xff\xff");
    }

    #[test]
    fn debug_shows_the_bytes_escaped_as_ascii() {
        let field = Utf8String::from_slice(b"Gr\x9fn");

        assert_eq!(format!("{field:?}"), r#"Utf8String("Gr\x9fn")"#);
    }
}
