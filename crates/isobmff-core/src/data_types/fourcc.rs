//! [`FourCc`], a four-character code

use core::fmt;

/// Four-character code such as the `type` field of a box or a brand, ISO/IEC 14496-12 §4.2.2
///
/// Any four bytes form a valid code. §4.2.2 has a code expressed as four
/// printable characters where it can be, and does not require it.
///
/// [`Display`](fmt::Display) renders the bytes through
/// [`escape_ascii`](slice::escape_ascii), so the text is reversible: two codes that
/// differ never print the same string.
///
/// # Examples
///
/// ```
/// use isobmff_core::FourCc;
///
/// // A code is four raw bytes
/// let code = FourCc::new(*b"moov");
/// assert_eq!(code.as_bytes(), b"moov");
///
/// // Bytes outside printable ASCII survive Display as escapes
/// let copyright = FourCc::new([0xa9, b'n', b'a', b'm']);
/// assert_eq!(copyright.to_string(), r"\xa9nam");
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FourCc([u8; 4]);

impl FourCc {
    /// Creates a code from its four bytes
    #[must_use]
    pub const fn new(bytes: [u8; 4]) -> Self {
        Self(bytes)
    }

    /// Returns the four bytes of the code
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 4] {
        &self.0
    }
}

impl fmt::Display for FourCc {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0.escape_ascii())
    }
}

impl fmt::Debug for FourCc {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "FourCc(\"{}\")", self.0.escape_ascii())
    }
}

#[cfg(test)]
mod tests {
    use alloc::format;
    use alloc::string::ToString as _;

    use super::FourCc;

    #[test]
    fn display_keeps_an_escape_distinct_from_the_characters_spelling_it() {
        let spelled_out = FourCc::new(*b"\\x00");
        let escaped = FourCc::new([0x00, b'x', b'0', b'0']);

        assert_ne!(spelled_out.to_string(), escaped.to_string());
    }

    #[test]
    fn debug_quotes_the_escaped_code() {
        let code = FourCc::new([0xa9, b'd', b'a', b'y']);

        assert_eq!(format!("{code:?}"), r#"FourCc("\xa9day")"#);
    }
}
