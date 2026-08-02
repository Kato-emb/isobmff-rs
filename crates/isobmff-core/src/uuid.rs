//! [`Uuid`], the `usertype` of an extended box type of ISO/IEC 14496-12 §4.2

use core::fmt;

/// Sixteen-byte UUID carried as the `usertype` of a box
///
/// Any sixteen bytes form a valid value; neither the RFC 4122 variant nor the
/// version field is checked.
///
/// [`Display`](fmt::Display) renders the RFC 4122 hyphenated form in lowercase
/// hexadecimal.
///
/// # Examples
///
/// ```
/// use isobmff_core::Uuid;
///
/// // A UUID is sixteen raw bytes
/// let user_type = Uuid::new([
///     0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
///     0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
/// ]);
///
/// // Display groups them as 8-4-4-4-12 hexadecimal digits
/// assert_eq!(user_type.to_string(), "01234567-89ab-cdef-0123-456789abcdef");
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Uuid([u8; 16]);

impl Uuid {
    /// Creates a UUID from its sixteen bytes
    #[must_use]
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Returns the sixteen bytes of the UUID
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Display for Uuid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (position, byte) in self.0.iter().enumerate() {
            if matches!(position, 4 | 6 | 8 | 10) {
                formatter.write_str("-")?;
            }

            write!(formatter, "{byte:02x}")?;
        }

        Ok(())
    }
}

impl fmt::Debug for Uuid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Uuid({self})")
    }
}

impl From<[u8; 16]> for Uuid {
    fn from(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

impl From<Uuid> for [u8; 16] {
    fn from(uuid: Uuid) -> Self {
        uuid.0
    }
}

#[cfg(test)]
mod tests {
    use super::Uuid;

    const SAMPLE: Uuid = Uuid::new([
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32,
        0x10,
    ]);

    #[test]
    fn display_renders_the_hyphenated_lowercase_form() {
        assert_eq!(SAMPLE.to_string(), "01234567-89ab-cdef-fedc-ba9876543210");
    }

    #[test]
    fn debug_wraps_the_hyphenated_form() {
        assert_eq!(
            format!("{SAMPLE:?}"),
            "Uuid(01234567-89ab-cdef-fedc-ba9876543210)"
        );
    }
}
