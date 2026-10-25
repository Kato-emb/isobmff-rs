//! [`Base64String`], null-terminated base64 encoded data

use alloc::string::String;
use core::fmt;

use super::utf8_string::Utf8String;
use crate::error::Error;

/// Field of the basic type `base64string`, ISO/IEC 14496-12 §4.2.1
///
/// The specification defines the type as base64 encoded data ended by a NUL,
/// which shall be present. The value holds the base64 text before the
/// terminator, not the data it encodes: decoding it is left to the caller.
/// Base64 is the encoding of IETF RFC 4648 §4: characters of its alphabet in
/// groups of four, the last group padded with `=`.
///
/// Reading is the lenient half, as for [`Utf8String`](crate::Utf8String):
/// [`from_slice`](Self::from_slice) accepts a field whose terminator is left
/// off and bytes that are not base64, and [`as_str`](Self::as_str) returns
/// the text whatever it is, where it is UTF-8. Writing is strict: it writes
/// the bytes and a terminator after them only where the bytes are base64.
///
/// # Examples
///
/// ```
/// use isobmff_core::Base64String;
///
/// // A field that ends at its terminator
/// let info = Base64String::from_slice(b"AAEC\0");
/// assert_eq!(info.as_str(), Some("AAEC"));
///
/// // Writing puts the terminator back
/// let mut buffer = vec![0xff; 5];
/// assert!(info.encode(&mut buffer).unwrap().is_empty());
/// assert_eq!(buffer, b"AAEC\0");
///
/// // Text that is not base64 is kept as read, and is not written
/// let garbled = Base64String::from_slice(b"AA*C\0");
/// assert_eq!(garbled.as_str(), Some("AA*C"));
/// assert!(garbled.encode(&mut [0; 5]).is_err());
///
/// // Only base64 text builds a field
/// assert_eq!(Base64String::new(String::from("AAEC")), Some(info));
/// assert_eq!(Base64String::new(String::from("AAE")), None);
/// ```
#[doc(alias = "base64string")]
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct Base64String(Utf8String);

impl Base64String {
    /// Creates the field from the base64 text it carries
    ///
    /// Returns `None` when `value` is not base64. The field that carries no
    /// data is [`default`](Self::default): the empty text, one NUL byte on the
    /// wire.
    #[must_use]
    pub fn new(value: String) -> Option<Self> {
        if !is_base64(value.as_bytes()) {
            return None;
        }

        Utf8String::new(value).map(Self)
    }

    /// Returns the text the field carries, terminator excluded, or `None` where its bytes are not UTF-8
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        self.0.as_str()
    }

    /// Returns the bytes the field carries, terminator excluded
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Reads the field that occupies the whole of `bytes`
    ///
    /// The field runs to the first NUL, or to the end of `bytes` where there is
    /// none. Bytes after a terminator are dropped.
    #[must_use]
    pub fn from_slice(bytes: &[u8]) -> Self {
        Self(Utf8String::from_slice(bytes))
    }

    /// Returns the length the field occupies, terminator included
    #[must_use]
    pub fn encoded_len(&self) -> u64 {
        self.0.encoded_len()
    }

    /// Writes the field and its terminator into the front of `buffer` and returns what is left
    ///
    /// # Errors
    ///
    /// * [`InvalidBase64String`](crate::ErrorKind::InvalidBase64String): the
    ///   bytes the field carries are not base64.
    /// * [`TruncatedBuffer`](crate::ErrorKind::TruncatedBuffer): `buffer` is shorter
    ///   than [`encoded_len`](Self::encoded_len).
    pub fn encode<'buffer>(&self, buffer: &'buffer mut [u8]) -> Result<&'buffer mut [u8], Error> {
        if !is_base64(self.as_bytes()) {
            return Err(Error::invalid_base64_string());
        }

        self.0.encode(buffer)
    }
}

impl fmt::Debug for Base64String {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Base64String(\"{}\")",
            self.as_bytes().escape_ascii()
        )
    }
}

/// Returns whether `bytes` are base64 text, IETF RFC 4648 §4, padding included
fn is_base64(bytes: &[u8]) -> bool {
    let text = bytes
        .strip_suffix(b"==")
        .or_else(|| bytes.strip_suffix(b"="))
        .unwrap_or(bytes);

    bytes.len().is_multiple_of(4)
        && text
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'+' | b'/'))
}

#[cfg(test)]
mod tests {
    use alloc::format;
    use alloc::string::String;

    use super::Base64String;
    use crate::error::Error;

    #[test]
    fn a_field_reads_to_its_terminator() {
        assert_eq!(
            Base64String::from_slice(b"AAEC\0\0\x01"),
            Base64String::new(String::from("AAEC")).unwrap()
        );
    }

    #[test]
    fn a_field_without_its_terminator_runs_to_the_end_of_the_field() {
        assert_eq!(
            Base64String::from_slice(b"AAEC"),
            Base64String::new(String::from("AAEC")).unwrap()
        );
    }

    #[test]
    fn a_field_holding_only_a_terminator_carries_no_data() {
        let field = Base64String::from_slice(b"\0");
        let mut buffer = [0xff; 1];

        assert_eq!(field, Base64String::default());
        assert!(field.encode(&mut buffer).unwrap().is_empty());
        assert_eq!(buffer, [0]);
    }

    #[test]
    fn text_with_one_or_two_padding_characters_is_base64() {
        for text in ["AAE=", "AA==", "AAECAw==", "+/09"] {
            assert!(Base64String::new(String::from(text)).is_some(), "{text}");
        }
    }

    #[test]
    fn text_outside_the_alphabet_or_its_padding_is_not_base64() {
        for text in ["AAE", "A===", "AA=A", "A=A=AAEC", "AA-_", "AA E", "AAEC\n"] {
            assert_eq!(Base64String::new(String::from(text)), None, "{text}");
        }
    }

    #[test]
    fn text_that_is_not_base64_is_kept_and_refused_on_writing() {
        let field = Base64String::from_slice(b"AA-_\0");
        let mut buffer = [0xaa; 5];

        assert_eq!(field.as_bytes(), b"AA-_");
        assert_eq!(
            field.encode(&mut buffer),
            Err(Error::invalid_base64_string())
        );
        assert_eq!(buffer, [0xaa; 5]);
    }

    #[test]
    fn debug_shows_the_bytes_escaped_as_ascii() {
        let field = Base64String::from_slice(b"AA\x9f");

        assert_eq!(format!("{field:?}"), r#"Base64String("AA\x9f")"#);
    }
}
