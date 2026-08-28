//! [`CompressorName`], the `compressorname` field of ISO/IEC 14496-12 §12.1.3

use core::fmt;

/// Name of the compressor a visual sample entry states, for information only
///
/// §12.1.3.3 lays the field out in a fixed 32 bytes: the first byte counts the
/// bytes to be displayed, that many bytes of displayable data follow, and
/// padding completes the 32. The field may be all zero.
///
/// The value holds the 32 bytes as they lie, so a field reads back exactly as
/// it was written even when its count exceeds the 31 bytes that follow it;
/// [`displayed`](Self::displayed) caps the count there, as a display must.
///
/// # Examples
///
/// ```
/// use isobmff_core::CompressorName;
///
/// // A name is padded to the fixed width behind its count
/// let name = CompressorName::new(b"AVC Coding").unwrap();
/// assert_eq!(name.displayed(), b"AVC Coding");
/// assert_eq!(name.as_bytes()[0], 10);
/// assert_eq!(name.as_bytes().len(), 32);
///
/// // A field set to zero names nothing
/// assert_eq!(CompressorName::EMPTY.displayed(), b"");
///
/// // A name longer than the 31 bytes the field leaves does not fit
/// assert_eq!(CompressorName::new(&[b'x'; 32]), None);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CompressorName([u8; Self::LEN]);

impl CompressorName {
    /// Length of the field, count byte included
    pub const LEN: usize = 32;

    /// Field set to zero, which the spec allows
    pub const EMPTY: Self = Self([0; Self::LEN]);

    /// Creates the field from the bytes to be displayed
    ///
    /// Returns `None` when `displayed` is longer than the 31 bytes the field
    /// has room for behind its count.
    #[must_use]
    pub fn new(displayed: &[u8]) -> Option<Self> {
        let mut bytes = [0; Self::LEN];
        bytes[0] = u8::try_from(displayed.len()).ok()?;
        bytes
            .get_mut(1..=displayed.len())?
            .copy_from_slice(displayed);

        Some(Self(bytes))
    }

    /// Creates the field from the 32 bytes it lies as
    #[must_use]
    pub const fn from_bytes(bytes: [u8; Self::LEN]) -> Self {
        Self(bytes)
    }

    /// Returns the 32 bytes the field lies as
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; Self::LEN] {
        &self.0
    }

    /// Returns the bytes to be displayed, as many as the count states and the
    /// field holds
    #[must_use]
    pub fn displayed(&self) -> &[u8] {
        let count = usize::from(self.0[0]).min(Self::LEN - 1);

        self.0.get(1..=count).unwrap_or(&[])
    }
}

impl fmt::Debug for CompressorName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "CompressorName(\"{}\")",
            self.displayed().escape_ascii()
        )
    }
}

#[cfg(test)]
mod tests {
    use alloc::format;

    use super::CompressorName;

    #[test]
    fn a_count_past_the_field_displays_the_bytes_the_field_holds() {
        let mut bytes = [b'x'; CompressorName::LEN];
        bytes[0] = 0xff;

        let name = CompressorName::from_bytes(bytes);

        assert_eq!(name.displayed(), &[b'x'; CompressorName::LEN - 1]);
        assert_eq!(name.as_bytes(), &bytes);
    }

    #[test]
    fn the_longest_name_that_fits_fills_the_field() {
        let name = CompressorName::new(&[b'x'; CompressorName::LEN - 1]).unwrap();

        assert_eq!(name.as_bytes()[0], 31);
        assert_eq!(name.displayed(), &[b'x'; CompressorName::LEN - 1]);
    }

    #[test]
    fn debug_shows_the_displayed_bytes_escaped() {
        let name = CompressorName::new(b"H.264\xa9").unwrap();

        assert_eq!(format!("{name:?}"), r#"CompressorName("H.264\xa9")"#);
    }
}
