//! [`CompressorName`], a name of up to 31 bytes in a field of 32

use core::str;

/// Name of the compressor that a [`VisualSampleEntry`](crate::VisualSampleEntry) gives for informative purposes, ISO/IEC 14496-12 §12.1.3
///
/// The field is 32 bytes: the first counts the bytes displayed, which follow
/// it as UTF-8, and padding completes the 32. A field read from a file is
/// kept as its 32 bytes, as found, whether or not its first byte counts what
/// follows and the bytes counted are UTF-8.
///
/// [`Default`] is the field set to 0.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::CompressorName;
///
/// // A name is built from the text displayed, which takes 31 bytes at most
/// let name = CompressorName::new("AVC Coding").unwrap();
/// assert_eq!(name.as_str(), Some("AVC Coding"));
/// assert_eq!(&name.to_bytes()[..11], b"\x0aAVC Coding");
/// assert_eq!(CompressorName::new(&"x".repeat(32)), None);
///
/// // The field set to 0 displays nothing
/// assert_eq!(CompressorName::default().as_bytes(), b"");
/// ```
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct CompressorName([u8; 32]);

impl CompressorName {
    /// Creates the field that displays `name`, or `None` if `name` takes more than 31 bytes
    #[must_use]
    pub fn new(name: &str) -> Option<Self> {
        let length = u8::try_from(name.len())
            .ok()
            .filter(|length| *length < 32)?;
        let mut bytes = [0; 32];
        let (count, text) = bytes.split_first_mut()?;
        *count = length;
        text.get_mut(..name.len())?.copy_from_slice(name.as_bytes());

        Some(Self(bytes))
    }

    /// Creates the field from the 32 bytes it occupies, as found
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the 32 bytes the field occupies
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 32] {
        self.0
    }

    /// Returns the bytes displayed: as many after the first as it counts, and at most the 31 there are
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        let (count, text) = self.0.split_first().unwrap_or((&0, &[]));
        text.get(..usize::from(*count)).unwrap_or(text)
    }

    /// Returns the text displayed, or `None` where the bytes displayed are not UTF-8
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        str::from_utf8(self.as_bytes()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::CompressorName;

    #[test]
    fn a_name_is_counted_by_its_first_byte_and_padded_with_zeros() {
        let mut expected = [0; 32];
        expected.get_mut(..5).unwrap().copy_from_slice(b"\x04GPAC");

        assert_eq!(
            CompressorName::new("GPAC").map(CompressorName::to_bytes),
            Some(expected)
        );
    }

    #[test]
    fn a_name_of_31_bytes_fills_the_field() {
        let name = "x".repeat(31);

        assert_eq!(
            CompressorName::new(&name).unwrap().as_str(),
            Some(name.as_str())
        );
    }

    #[test]
    fn a_count_past_the_field_displays_the_31_bytes_there_are() {
        let field = CompressorName::from_bytes([0xff; 32]);

        assert_eq!(field.as_bytes(), [0xff; 31]);
    }

    #[test]
    fn bytes_displayed_that_are_not_utf8_have_no_text() {
        let mut bytes = [0; 32];
        bytes.get_mut(..3).unwrap().copy_from_slice(b"\x02\x9f\x9f");

        assert_eq!(CompressorName::from_bytes(bytes).as_str(), None);
    }
}
