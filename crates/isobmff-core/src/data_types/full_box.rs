//! [`FullBoxFields`] and [`FullBoxFlags`], the `version` and `flags` of ISO/IEC 14496-12 §4.2

/// Widest value the 24-bit `flags` field carries
const FLAGS_MAXIMUM: u32 = 0x00FF_FFFF;

/// `flags` field of a full box
///
/// The field is 24 bits wide and is carried in the low bits of a `u32`. A value
/// too wide for it is not a flags value at all: [`new`](Self::new) refuses it
/// rather than dropping the bits that overflow.
///
/// What a bit means is settled by the box that carries it, so the field has no
/// named bits of its own.
///
/// # Examples
///
/// ```
/// use isobmff_core::FullBoxFlags;
///
/// // A value the field carries
/// let flags = FullBoxFlags::new(0x0000_0123).unwrap();
/// assert_eq!(flags.bits(), 0x0000_0123);
///
/// // A value too wide for the field is refused
/// assert_eq!(FullBoxFlags::new(0x1234_5678), None);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FullBoxFlags(u32);

impl FullBoxFlags {
    /// Flags with no bit set, which a box defining none of them carries
    pub const ZERO: Self = Self(0);

    /// Creates flags from the value of the `flags` field
    ///
    /// Returns `None` above `0x00FF_FFFF`, which the 24-bit field cannot carry.
    #[must_use]
    pub const fn new(bits: u32) -> Option<Self> {
        if bits > FLAGS_MAXIMUM {
            return None;
        }

        Some(Self(bits))
    }

    /// Returns the flags as the low 24 bits of a `u32`
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }
}

/// Fields a full box adds ahead of its payload
///
/// A box the spec declares as a `FullBox` opens its payload with a `version`
/// and 24 bits of `flags`, four bytes in all; the header in front of that is a
/// [`BoxHeader`](crate::BoxHeader) like any other box's. Which fields a version
/// selects, and what a flag turns on, belong to the box that declares them.
///
/// # Examples
///
/// ```
/// use isobmff_core::{FullBoxFields, FullBoxFlags};
///
/// // A full box payload opens with the four bytes of these fields
/// let payload = b"\x01\x00\x00\x07and the fields of the box";
/// let (word, rest) = payload.split_first_chunk::<4>().unwrap();
///
/// // Version and flags are read together, and what follows is the box's own
/// let full_box = FullBoxFields::from_bytes(word);
/// assert_eq!(full_box.version(), 1);
/// assert_eq!(full_box.flags(), FullBoxFlags::new(7).unwrap());
/// assert_eq!(rest, b"and the fields of the box");
///
/// // Written back, the fields yield the bytes they were read from
/// assert_eq!(full_box.to_bytes(), *word);
///
/// // A box that declares no flags of its own leaves them clear
/// assert_eq!(FullBoxFields::new(1, FullBoxFlags::ZERO).to_bytes(), [1, 0, 0, 0]);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FullBoxFields {
    version: u8,
    flags: FullBoxFlags,
}

impl FullBoxFields {
    /// Creates the fields from a version and flags
    #[must_use]
    pub const fn new(version: u8, flags: FullBoxFlags) -> Self {
        Self { version, flags }
    }

    /// Returns the version the box declares
    #[must_use]
    pub const fn version(self) -> u8 {
        self.version
    }

    /// Returns the flags the box declares
    #[must_use]
    pub const fn flags(self) -> FullBoxFlags {
        self.flags
    }

    /// Reads the fields from the four bytes that open a full box payload
    ///
    /// Every four bytes read as a version and flags, so the fields the payload
    /// carries after them are what a box can still refuse.
    #[must_use]
    pub const fn from_bytes(bytes: &[u8; 4]) -> Self {
        let [version, high, middle, low] = *bytes;

        Self {
            version,
            flags: FullBoxFlags(u32::from_be_bytes([0, high, middle, low])),
        }
    }

    /// Returns the four bytes the fields occupy on the wire
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 4] {
        let [_unused, high, middle, low] = self.flags.bits().to_be_bytes();

        [self.version, high, middle, low]
    }
}

#[cfg(test)]
mod tests {
    use super::{FullBoxFields, FullBoxFlags};

    /// The word with each of its two fields empty and full in turn
    const EVERY_FIELD_BOUNDARY: [[u8; 4]; 4] = [
        [0x00, 0x00, 0x00, 0x00],
        [0xff, 0xff, 0xff, 0xff],
        [0xff, 0x00, 0x00, 0x00],
        [0x00, 0xff, 0xff, 0xff],
    ];

    #[test]
    fn flags_wider_than_the_field_are_rejected() {
        assert_eq!(FullBoxFlags::new(0x0100_0000), None);
    }

    #[test]
    fn flags_of_the_full_field_width_are_accepted() {
        assert_eq!(
            FullBoxFlags::new(0x00FF_FFFF).map(FullBoxFlags::bits),
            Some(0x00FF_FFFF)
        );
    }

    #[test]
    fn a_word_takes_its_version_from_the_first_byte_and_its_flags_from_the_rest() {
        let fields = FullBoxFields::from_bytes(&[0x01, 0x00, 0x12, 0x34]);

        assert_eq!(
            fields,
            FullBoxFields::new(1, FullBoxFlags::new(0x0000_1234).unwrap())
        );
    }

    #[test]
    fn a_word_at_any_field_boundary_writes_back_the_bytes_it_was_read_from() {
        for word in EVERY_FIELD_BOUNDARY {
            assert_eq!(FullBoxFields::from_bytes(&word).to_bytes(), word);
        }
    }
}
