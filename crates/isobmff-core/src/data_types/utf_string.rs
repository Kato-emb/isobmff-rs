//! [`UtfString`], a null-terminated UTF-8 or UTF-16 string

use alloc::borrow::Cow;
use alloc::string::String;
use alloc::vec::Vec;
use core::{char, fmt, str};

use crate::error::Error;

const BIG_ENDIAN_BYTE_ORDER_MARK: [u8; 2] = [0xfe, 0xff];
const LITTLE_ENDIAN_BYTE_ORDER_MARK: [u8; 2] = [0xff, 0xfe];

/// Field of the basic type `utfstring`, ISO/IEC 14496-12 §4.2.1
///
/// The specification defines the type as a string in UTF-8, ended by a NUL,
/// or in UTF-16, opened by a byte order mark and ended by two NUL bytes. The
/// value holds the bytes before the terminator as found, byte order mark
/// included, so a string is written back in the encoding it was read in. The
/// byte order mark tells the encodings apart: UTF-8 holds no `0xfe` or `0xff`
/// byte, so no UTF-8 string opens with one.
///
/// Reading is the lenient half, as for [`Utf8String`](crate::Utf8String):
/// [`from_slice`](Self::from_slice) accepts a string whose terminator is left
/// off and bytes in neither encoding, and [`to_str`](Self::to_str) returns
/// the text where the bytes are in the encoding their opening names. Writing
/// is strict: it writes the bytes and a terminator after them only where they
/// are.
///
/// # Examples
///
/// ```
/// use isobmff_core::UtfString;
///
/// // A UTF-8 string
/// let notice = UtfString::from_slice(b"(c) 2026\0");
/// assert_eq!(notice.to_str().as_deref(), Some("(c) 2026"));
///
/// // A big-endian UTF-16 string, ended by two NUL bytes
/// let utf16 = UtfString::from_slice(b"\xfe\xff\0(\0c\0)\0\0");
/// assert_eq!(utf16.to_str().as_deref(), Some("(c)"));
///
/// // It is written back as UTF-16, the terminator included
/// assert_eq!(utf16.encoded_len(), 10);
///
/// let mut buffer = vec![0xff; 10];
/// assert!(utf16.encode(&mut buffer).unwrap().is_empty());
/// assert_eq!(buffer, b"\xfe\xff\0(\0c\0)\0\0");
///
/// // A string built from its text is UTF-8, and may not hold a NUL
/// assert_eq!(
///     UtfString::new(String::from("(c) 2026")),
///     Some(notice)
/// );
/// assert_eq!(UtfString::new(String::from("(c)\0")), None);
/// ```
#[doc(alias = "utfstring")]
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct UtfString(Vec<u8>);

impl UtfString {
    /// Creates the field from the text it carries, in UTF-8
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

    /// Returns the text the field carries, terminator and byte order mark excluded
    ///
    /// The text is borrowed where the field is in UTF-8 and decoded where it is
    /// in UTF-16. Returns `None` where its bytes are not in the encoding their
    /// opening names.
    #[must_use]
    pub fn to_str(&self) -> Option<Cow<'_, str>> {
        let (Some(decode_unit), Some((_, units))) =
            (utf16_decode_unit(&self.0), self.0.split_first_chunk::<2>())
        else {
            return str::from_utf8(&self.0).ok().map(Cow::Borrowed);
        };

        let (units, []) = units.as_chunks::<2>() else {
            return None;
        };

        char::decode_utf16(units.iter().map(|unit| decode_unit(*unit)))
            .collect::<Result<String, _>>()
            .ok()
            .map(Cow::Owned)
    }

    /// Returns the bytes the field carries, terminator excluded and byte order mark included
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Reads the field that occupies the whole of `bytes`
    ///
    /// A field that opens with a byte order mark is UTF-16 and runs to the
    /// first code unit of two NUL bytes; any other runs to the first NUL. Either
    /// runs to the end of `bytes` where it has no terminator. Bytes after a
    /// terminator are dropped: this reads a field that is the last of its box,
    /// so nothing that follows lays claim to them.
    #[must_use]
    pub fn from_slice(bytes: &[u8]) -> Self {
        let text = if utf16_decode_unit(bytes).is_some() {
            let terminator = bytes
                .chunks(2)
                .position(|unit| unit == [0, 0])
                .and_then(|units| units.checked_mul(2));

            terminator.and_then(|end| bytes.get(..end)).unwrap_or(bytes)
        } else {
            bytes.split(|byte| *byte == 0).next().unwrap_or_default()
        };

        Self(text.to_vec())
    }

    /// Returns the length the field occupies, terminator included
    #[must_use]
    pub fn encoded_len(&self) -> u64 {
        u64::try_from(self.0.len().saturating_add(self.terminator().len())).unwrap_or(u64::MAX)
    }

    /// Writes the field and its terminator into the front of `buffer` and returns what is left
    ///
    /// # Errors
    ///
    /// * [`InvalidUtfString`](crate::ErrorKind::InvalidUtfString): the bytes
    ///   the field carries are not in the encoding their opening names.
    /// * [`TruncatedBuffer`](crate::ErrorKind::TruncatedBuffer): `buffer` is shorter
    ///   than [`encoded_len`](Self::encoded_len).
    pub fn encode<'buffer>(&self, buffer: &'buffer mut [u8]) -> Result<&'buffer mut [u8], Error> {
        if self.to_str().is_none() {
            return Err(Error::invalid_utf_string());
        }

        let terminator = self.terminator();
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
        let (end, rest) = rest
            .split_at_mut_checked(terminator.len())
            .ok_or_else(too_short)?;

        text.copy_from_slice(&self.0);
        end.copy_from_slice(terminator);

        Ok(rest)
    }

    fn terminator(&self) -> &'static [u8] {
        if utf16_decode_unit(&self.0).is_some() {
            &[0, 0]
        } else {
            &[0]
        }
    }
}

/// Returns how to read a code unit of `bytes`, or `None` where they open with no byte order mark and so are UTF-8
fn utf16_decode_unit(bytes: &[u8]) -> Option<fn([u8; 2]) -> u16> {
    match *bytes.first_chunk::<2>()? {
        BIG_ENDIAN_BYTE_ORDER_MARK => Some(u16::from_be_bytes),
        LITTLE_ENDIAN_BYTE_ORDER_MARK => Some(u16::from_le_bytes),
        _ => None,
    }
}

impl fmt::Debug for UtfString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "UtfString(\"{}\")", self.0.escape_ascii())
    }
}

#[cfg(test)]
mod tests {
    use alloc::format;
    use alloc::string::String;

    use super::UtfString;
    use crate::error::Error;

    #[test]
    fn a_utf8_string_reads_to_its_terminator() {
        let field = UtfString::from_slice(b"(c) 2026\0trailing");

        assert_eq!(field, UtfString(b"(c) 2026".to_vec()));
        assert_eq!(field.to_str().as_deref(), Some("(c) 2026"));
    }

    #[test]
    fn a_big_endian_utf16_string_reads_to_its_two_byte_terminator() {
        let field = UtfString::from_slice(b"\xfe\xff\x65\xe5\0a\0\0trailing");

        assert_eq!(field, UtfString(b"\xfe\xff\x65\xe5\0a".to_vec()));
        assert_eq!(field.to_str().as_deref(), Some("日a"));
    }

    #[test]
    fn a_little_endian_utf16_string_reads_to_its_two_byte_terminator() {
        let field = UtfString::from_slice(b"\xff\xfe\xe5\x65a\0\0\0");

        assert_eq!(field, UtfString(b"\xff\xfe\xe5\x65a\0".to_vec()));
        assert_eq!(field.to_str().as_deref(), Some("日a"));
    }

    #[test]
    fn a_nul_byte_inside_a_utf16_code_unit_does_not_end_the_string() {
        let field = UtfString::from_slice(b"\xfe\xff\0a\0\0");

        assert_eq!(field.to_str().as_deref(), Some("a"));
    }

    #[test]
    fn a_string_without_its_terminator_runs_to_the_end_of_the_field() {
        assert_eq!(
            UtfString::from_slice(b"(c) 2026"),
            UtfString(b"(c) 2026".to_vec())
        );
        assert_eq!(
            UtfString::from_slice(b"\xfe\xff\0a"),
            UtfString(b"\xfe\xff\0a".to_vec())
        );
    }

    #[test]
    fn an_empty_field_reads_as_the_empty_utf8_string() {
        assert_eq!(UtfString::from_slice(b""), UtfString::default());
        assert_eq!(UtfString::from_slice(b"\0"), UtfString::default());
    }

    #[test]
    fn a_utf16_string_is_written_back_in_its_byte_order_with_two_terminating_bytes() {
        for read in [b"\xfe\xff\0a\0\0".as_slice(), b"\xff\xfea\0\0\0"] {
            let field = UtfString::from_slice(read);
            let mut buffer = [0xaa; 6];

            let rest = field.encode(&mut buffer).unwrap();

            assert!(rest.is_empty());
            assert_eq!(field.encoded_len(), 6);
            assert_eq!(buffer.as_slice(), read);
        }
    }

    #[test]
    fn a_utf8_string_is_written_with_one_terminating_byte() {
        let field = UtfString::new(String::from("ab")).unwrap();
        let mut buffer = [0xff; 5];

        let rest = field.encode(&mut buffer).unwrap();

        assert_eq!(rest, [0xff; 2]);
        assert_eq!(field.encoded_len(), 3);
        assert_eq!(buffer, *b"ab\0\xff\xff");
    }

    #[test]
    fn bytes_in_neither_encoding_are_kept_and_refused_on_writing() {
        let unpaired_surrogate = b"\xfe\xff\xd8\0".as_slice();
        let odd_length = b"\xfe\xff\0".as_slice();
        let not_utf8 = b"\x9f".as_slice();

        for read in [unpaired_surrogate, odd_length, not_utf8] {
            let field = UtfString::from_slice(read);
            let mut buffer = [0xaa; 6];

            assert_eq!(field.as_bytes(), read);
            assert_eq!(field.to_str(), None);
            assert_eq!(field.encode(&mut buffer), Err(Error::invalid_utf_string()));
            assert_eq!(buffer, [0xaa; 6]);
        }
    }

    #[test]
    fn a_buffer_one_byte_short_of_the_utf16_terminator_is_refused() {
        let field = UtfString::from_slice(b"\xfe\xff\0a");

        assert_eq!(
            field.encode(&mut [0; 5]),
            Err(Error::truncated_buffer(6, 5))
        );
    }

    #[test]
    fn debug_shows_the_bytes_escaped_as_ascii() {
        let field = UtfString::from_slice(b"\xfe\xff\0a");

        assert_eq!(format!("{field:?}"), r#"UtfString("\xfe\xff\x00a")"#);
    }
}
