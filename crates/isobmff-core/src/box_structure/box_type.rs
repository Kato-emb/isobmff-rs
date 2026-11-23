//! [`BoxType`], [`CompactType`] and [`ExtendedType`], the type a box header declares

use core::fmt;

use crate::data_types::fourcc::FourCc;
use crate::data_types::uuid::Uuid;

/// Four-character code that introduces an extended type instead of naming a box
const EXTENDED_MARKER: FourCc = FourCc::new(*b"uuid");

/// Bits of a UUID that follow the four-character code it maps
const RESERVED_MASK: u128 = 0x0000_0000_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF;

/// Twelve bytes ISO/IEC 14496-12 §4.2.3 reserves after the four-character code a UUID maps
const RESERVED_SUFFIX: u128 = 0x0000_0000_0011_0010_8000_00AA_0038_9B71;

/// Four-character code that names a box, ISO/IEC 14496-12 §4.2.2
///
/// Excludes the reserved `uuid` code, which introduces an
/// [`ExtendedType`] instead of naming a box.
///
/// # Examples
///
/// ```
/// use isobmff_core::{CompactType, FourCc};
///
/// // Any code but the reserved one names a box
/// let moov = CompactType::new(FourCc::new(*b"moov")).unwrap();
/// assert_eq!(moov.four_cc(), FourCc::new(*b"moov"));
///
/// // The reserved code does not
/// assert_eq!(CompactType::new(FourCc::new(*b"uuid")), None);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct CompactType(FourCc);

impl CompactType {
    /// Creates a compact type from a four-character code
    ///
    /// Returns `None` for the reserved `uuid` code.
    #[must_use]
    pub const fn new(four_cc: FourCc) -> Option<Self> {
        match four_cc {
            EXTENDED_MARKER => None,
            _ => Some(Self(four_cc)),
        }
    }

    /// Returns the four-character code
    #[must_use]
    pub const fn four_cc(self) -> FourCc {
        self.0
    }
}

/// UUID that names a box whose `boxtype` is `uuid`, ISO/IEC 14496-12 §4.2.2
///
/// Excludes the UUIDs §4.2.3 reserves for mapping a compact type into the
/// UUID space — a four-character code followed by
/// `0011-0010-8000-00AA00389B71` — which the `extended_type` of a box shall
/// not equal. A header read off bytes keeps such a UUID as found, and is not
/// written back.
///
/// # Examples
///
/// ```
/// use isobmff_core::{ExtendedType, Uuid};
///
/// // A UUID of the vendor's own names a box
/// let vendor = Uuid::new([0xab; 16]);
/// assert_eq!(ExtendedType::new(vendor).map(ExtendedType::uuid), Some(vendor));
///
/// // The UUID that `moov` maps to is reserved
/// let mapped_moov = Uuid::new([
///     b'm', b'o', b'o', b'v', 0x00, 0x11, 0x00, 0x10,
///     0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
/// ]);
/// assert_eq!(ExtendedType::new(mapped_moov), None);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ExtendedType(Uuid);

impl ExtendedType {
    /// Creates an extended type from a UUID
    ///
    /// Returns `None` for a UUID of the form §4.2.3 reserves.
    #[must_use]
    pub const fn new(uuid: Uuid) -> Option<Self> {
        if maps_a_compact_type(uuid) {
            return None;
        }

        Some(Self(uuid))
    }

    /// Creates the extended type a header read off bytes declares, the reserved form included
    pub(crate) const fn as_found(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Returns the UUID
    #[must_use]
    pub const fn uuid(self) -> Uuid {
        self.0
    }
}

/// Returns whether `uuid` is the UUID ISO/IEC 14496-12 §4.2.3 maps a compact type to
const fn maps_a_compact_type(uuid: Uuid) -> bool {
    u128::from_be_bytes(*uuid.as_bytes()) & RESERVED_MASK == RESERVED_SUFFIX
}

/// Type a box header declares, ISO/IEC 14496-12 §4.2.2
///
/// A box is named either by a four-character code, or — when that code is the
/// reserved `uuid` — by the UUID of its `extended_type` field.
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxType, FourCc};
///
/// // A compact type puts its own code in the `boxtype` field
/// const MOOV: BoxType = BoxType::compact(*b"moov");
/// assert_eq!(MOOV.four_cc(), FourCc::new(*b"moov"));
///
/// // An extended type puts the reserved code there, and its UUID in `extended_type`
/// const VENDOR: BoxType = BoxType::extended([0xab; 16]);
/// assert_eq!(VENDOR.four_cc(), FourCc::new(*b"uuid"));
/// ```
#[expect(
    clippy::exhaustive_enums,
    reason = "ISO/IEC 14496-12 §4.2.2 closes box identity over exactly these two forms"
)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum BoxType {
    /// Box named by a four-character code
    Compact(CompactType),
    /// Box named by the UUID of its `extended_type` field
    Extended(ExtendedType),
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
    #[expect(
        clippy::panic,
        reason = "constructor for const definitions; run-time input takes the fallible CompactType::new"
    )]
    pub const fn compact(code: [u8; 4]) -> Self {
        match CompactType::new(FourCc::new(code)) {
            Some(compact) => Self::Compact(compact),
            None => panic!("`uuid` is reserved for extended box types"),
        }
    }

    /// Creates an extended box type from a UUID known at compile time
    ///
    /// # Panics
    ///
    /// Panics if `uuid` is of the form ISO/IEC 14496-12 §4.2.3 reserves; in a
    /// `const` context that is a compile-time error. For a UUID obtained at
    /// run time, use [`ExtendedType::new`], which reports the same condition
    /// as `None`.
    #[must_use]
    #[expect(
        clippy::panic,
        reason = "constructor for const definitions; run-time input takes the fallible ExtendedType::new"
    )]
    pub const fn extended(uuid: [u8; 16]) -> Self {
        match ExtendedType::new(Uuid::new(uuid)) {
            Some(extended) => Self::Extended(extended),
            None => panic!("UUIDs that map a compact type are reserved"),
        }
    }

    /// Returns the four-character code that the `boxtype` field carries
    ///
    /// The extended form yields the reserved `uuid`; its UUID is a separate
    /// field.
    #[must_use]
    pub const fn four_cc(self) -> FourCc {
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
            Self::Extended(extended) => write!(formatter, "uuid {}", extended.uuid()),
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString as _;

    use super::{BoxType, CompactType, ExtendedType};
    use crate::data_types::fourcc::FourCc;
    use crate::data_types::uuid::Uuid;

    const USER_TYPE: [u8; 16] = [
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32,
        0x10,
    ];

    /// UUID that `free` maps to under ISO/IEC 14496-12 §4.2.3
    const MAPPED_FREE: [u8; 16] = [
        b'f', b'r', b'e', b'e', 0x00, 0x11, 0x00, 0x10, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B,
        0x71,
    ];

    #[test]
    fn compact_type_accepts_a_code_that_is_not_printable() {
        let four_cc = FourCc::new([0xa9, b'n', b'a', b'm']);

        assert_eq!(
            CompactType::new(four_cc).map(CompactType::four_cc),
            Some(four_cc)
        );
    }

    #[test]
    fn extended_type_accepts_a_uuid_that_differs_from_the_reserved_form_in_one_byte() {
        let mut near_miss = MAPPED_FREE;
        let [.., last] = &mut near_miss;
        *last = 0x72;

        assert_eq!(
            ExtendedType::new(Uuid::new(near_miss)).map(ExtendedType::uuid),
            Some(Uuid::new(near_miss))
        );
    }

    #[test]
    fn the_const_constructors_build_what_the_checked_ones_build() {
        const MOOV: BoxType = BoxType::compact(*b"moov");
        const VENDOR: BoxType = BoxType::extended(USER_TYPE);

        assert_eq!(
            [MOOV, VENDOR],
            [
                BoxType::Compact(CompactType::new(FourCc::new(*b"moov")).unwrap()),
                BoxType::Extended(ExtendedType::new(Uuid::new(USER_TYPE)).unwrap()),
            ]
        );
    }

    #[test]
    #[should_panic(expected = "`uuid` is reserved for extended box types")]
    fn the_const_compact_constructor_rejects_the_code_reserved_for_extended_types() {
        let _ = BoxType::compact(*b"uuid");
    }

    #[test]
    #[should_panic(expected = "UUIDs that map a compact type are reserved")]
    fn the_const_extended_constructor_rejects_a_uuid_that_maps_a_compact_type() {
        let _ = BoxType::extended(MAPPED_FREE);
    }

    #[test]
    fn display_of_the_compact_form_is_the_code_alone() {
        assert_eq!(BoxType::compact(*b"moov").to_string(), "moov");
    }

    #[test]
    fn display_of_the_extended_form_names_the_marker_and_the_uuid() {
        assert_eq!(
            BoxType::extended(USER_TYPE).to_string(),
            "uuid 01234567-89ab-cdef-fedc-ba9876543210"
        );
    }
}
