//! [`BoxType`] and [`CompactType`], the box `type` of ISO/IEC 14496-12 §4.2

use core::fmt;

use crate::fourcc::FourCC;
use crate::uuid::Uuid;

/// Four-character code reserved to introduce a `usertype`
const EXTENDED_MARKER: FourCC = FourCC::new(*b"uuid");

/// Four-character code that names a box
///
/// Excludes the reserved `uuid` code, which introduces a `usertype` instead of
/// naming a box.
///
/// # Examples
///
/// ```
/// use isobmff_core::{CompactType, FourCC};
///
/// // Any code but the reserved one names a box
/// let moov = CompactType::new(FourCC::new(*b"moov")).unwrap();
/// assert_eq!(moov.four_cc(), FourCC::new(*b"moov"));
///
/// // The reserved code does not
/// assert_eq!(CompactType::new(FourCC::new(*b"uuid")), None);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct CompactType(FourCC);

impl CompactType {
    /// Creates a compact type from a four-character code
    ///
    /// Returns `None` for the reserved `uuid` code.
    #[must_use]
    pub const fn new(four_cc: FourCC) -> Option<Self> {
        match four_cc {
            EXTENDED_MARKER => None,
            _ => Some(Self(four_cc)),
        }
    }

    /// Returns the four-character code
    #[must_use]
    pub const fn four_cc(self) -> FourCC {
        self.0
    }
}

/// `type` field of a box
///
/// A box is identified either by a four-character code, or — when that code is the
/// reserved `uuid` — by the [`Uuid`] of its `usertype` field.
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxType, FourCC, Uuid};
///
/// // A compact type puts its own code in the `type` field
/// const MOOV: BoxType = BoxType::compact(*b"moov");
/// assert_eq!(MOOV.four_cc(), FourCC::new(*b"moov"));
///
/// // An extended type puts the reserved code there, and its UUID in `usertype`
/// let extended = BoxType::Extended(Uuid::new([0xab; 16]));
/// assert_eq!(extended.four_cc(), FourCC::new(*b"uuid"));
/// ```
#[allow(
    clippy::exhaustive_enums,
    reason = "ISO/IEC 14496-12 §4.2 closes box identity over exactly these two forms"
)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum BoxType {
    /// Box named by a four-character code
    Compact(CompactType),
    /// Box named by the UUID of its `usertype` field
    Extended(Uuid),
}

impl BoxType {
    /// Creates a compact box type from a four-character code known at compile time
    ///
    /// # Panics
    ///
    /// Panics if `code` is the reserved `uuid`; in a `const` context that is a
    /// compile-time error. For a code obtained at run time, use
    /// [`CompactType::new`], which reports the same condition as `None`.
    #[must_use]
    #[allow(
        clippy::panic,
        reason = "constructor for const definitions; run-time input takes the fallible CompactType::new"
    )]
    pub const fn compact(code: [u8; 4]) -> Self {
        match CompactType::new(FourCC::new(code)) {
            Some(compact) => Self::Compact(compact),
            None => panic!("`uuid` is reserved for extended box types"),
        }
    }

    /// Returns the four-character code that the `type` field carries
    ///
    /// The extended form yields the reserved `uuid`; its `usertype` is a separate
    /// field.
    #[must_use]
    pub const fn four_cc(self) -> FourCC {
        match self {
            Self::Compact(compact) => compact.four_cc(),
            Self::Extended(_) => EXTENDED_MARKER,
        }
    }
}

impl fmt::Display for BoxType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compact(compact) => fmt::Display::fmt(&compact.four_cc(), formatter),
            Self::Extended(user_type) => write!(formatter, "uuid {user_type}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BoxType, CompactType};
    use crate::fourcc::FourCC;
    use crate::uuid::Uuid;

    const USER_TYPE: Uuid = Uuid::new([
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32,
        0x10,
    ]);

    #[test]
    fn compact_type_rejects_the_code_reserved_for_user_types() {
        assert_eq!(CompactType::new(FourCC::new(*b"uuid")), None);
    }

    #[test]
    fn compact_type_accepts_a_code_that_is_not_printable() {
        let four_cc = FourCC::new([0xa9, b'n', b'a', b'm']);

        assert_eq!(
            CompactType::new(four_cc).map(CompactType::four_cc),
            Some(four_cc)
        );
    }

    #[test]
    fn the_const_constructor_builds_what_the_checked_one_builds() {
        const MOOV: BoxType = BoxType::compact(*b"moov");

        assert_eq!(
            MOOV,
            BoxType::Compact(CompactType::new(FourCC::new(*b"moov")).unwrap())
        );
    }

    #[test]
    #[should_panic(expected = "`uuid` is reserved for extended box types")]
    fn the_const_constructor_rejects_the_code_reserved_for_user_types() {
        let _ = BoxType::compact(*b"uuid");
    }

    #[test]
    fn the_compact_form_puts_its_own_code_in_the_type_field() {
        assert_eq!(BoxType::compact(*b"moov").four_cc(), FourCC::new(*b"moov"));
    }

    #[test]
    fn the_extended_form_puts_the_reserved_code_in_the_type_field() {
        assert_eq!(
            BoxType::Extended(USER_TYPE).four_cc(),
            FourCC::new(*b"uuid")
        );
    }

    #[test]
    fn display_of_the_compact_form_is_the_code_alone() {
        assert_eq!(BoxType::compact(*b"moov").to_string(), "moov");
    }

    #[test]
    fn display_of_the_extended_form_names_the_marker_and_the_user_type() {
        assert_eq!(
            BoxType::Extended(USER_TYPE).to_string(),
            "uuid 01234567-89ab-cdef-fedc-ba9876543210"
        );
    }
}
