//! [`BoxSize`], [`CompactSize`], and [`ExtendedSize`], the `size` and
//! `largesize` fields of ISO/IEC 14496-12 §4.2

/// Smallest total a compact size may declare: the `size` and `type` fields
const COMPACT_MINIMUM: u32 = 8;

/// Smallest total an extended size may declare: the compact minimum plus `largesize`
const EXTENDED_MINIMUM: u64 = 16;

/// Total byte count of a box, declared in the 32-bit `size` field
///
/// The total covers the header as well as the payload, so it is at least the
/// eight bytes of the `size` and `type` fields themselves.
///
/// # Examples
///
/// ```
/// use isobmff_core::CompactSize;
///
/// // A total that counts the fields it is stored in
/// let size = CompactSize::new(24).unwrap();
/// assert_eq!(size.get(), 24);
///
/// // A total smaller than the header it is part of
/// assert_eq!(CompactSize::new(4), None);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CompactSize(u32);

impl CompactSize {
    /// Creates a compact size from the value of the `size` field
    ///
    /// Returns `None` below eight, which no box can declare: the total counts
    /// the `size` and `type` fields it is stored in. The reserved values `0`
    /// and `1` — which select [`BoxSize::ToEndOfFile`] and
    /// [`BoxSize::Extended`] on the wire — fall below that floor and are
    /// rejected with it. Totals that fail to cover a `largesize` or `usertype`
    /// field are rejected by [`BoxHeader::new`](crate::BoxHeader::new).
    #[must_use]
    pub const fn new(size: u32) -> Option<Self> {
        if size < COMPACT_MINIMUM {
            return None;
        }

        Some(Self(size))
    }

    /// Returns the declared total
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Total byte count of a box, declared in the 64-bit `largesize` field
///
/// The total covers the header as well as the payload, so it is at least the
/// sixteen bytes of the `size`, `type`, and `largesize` fields themselves.
///
/// # Examples
///
/// ```
/// use isobmff_core::ExtendedSize;
///
/// // A total that counts the fields it is stored in
/// let size = ExtendedSize::new(5_000_000_000).unwrap();
/// assert_eq!(size.get(), 5_000_000_000);
///
/// // A total smaller than the header it is part of
/// assert_eq!(ExtendedSize::new(8), None);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ExtendedSize(u64);

impl ExtendedSize {
    /// Creates an extended size from the value of the `largesize` field
    ///
    /// Returns `None` below sixteen, which no box can declare: the total counts
    /// the `size`, `type`, and `largesize` fields it is stored in. Totals that
    /// fail to cover a `usertype` field are rejected by
    /// [`BoxHeader::new`](crate::BoxHeader::new).
    #[must_use]
    pub const fn new(size: u64) -> Option<Self> {
        if size < EXTENDED_MINIMUM {
            return None;
        }

        Some(Self(size))
    }

    /// Returns the declared total
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// `size` field of a box, in the form the wire carries it
///
/// A box states its total either in the 32-bit `size` field, or — when that
/// field holds the reserved `1` — in the 64-bit `largesize` field that follows
/// the `type`. The reserved `0` states no total at all: the box runs to the end
/// of the enclosing file.
///
/// The three forms stay distinct, so a box re-encodes to the bytes it was
/// decoded from. Choosing the shortest form that fits, or resolving
/// [`ToEndOfFile`](Self::ToEndOfFile) against an input length, is a policy the
/// caller applies.
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxSize, CompactSize, ExtendedSize};
///
/// // A total fits either width, and each width is a distinct form
/// let compact = BoxSize::Compact(CompactSize::new(24).unwrap());
/// let extended = BoxSize::Extended(ExtendedSize::new(24).unwrap());
/// assert_eq!(compact.total_bytes(), extended.total_bytes());
/// assert_ne!(compact, extended);
///
/// // The end-of-file form declares no total
/// assert_eq!(BoxSize::ToEndOfFile.total_bytes(), None);
/// ```
#[allow(
    clippy::exhaustive_enums,
    reason = "ISO/IEC 14496-12 §4.2 closes the size field over exactly these three forms"
)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BoxSize {
    /// Total declared in the `size` field
    Compact(CompactSize),
    /// Total declared in the `largesize` field
    #[doc(alias = "largesize")]
    Extended(ExtendedSize),
    /// Box running to the end of the enclosing file
    ToEndOfFile,
}

impl BoxSize {
    /// Returns the declared total, header included
    ///
    /// Returns `None` for [`ToEndOfFile`](Self::ToEndOfFile), which declares no
    /// total.
    #[must_use]
    pub const fn total_bytes(self) -> Option<u64> {
        match self {
            Self::Compact(size) => Some(size.get() as u64),
            Self::Extended(size) => Some(size.get()),
            Self::ToEndOfFile => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BoxSize, CompactSize, ExtendedSize};

    #[test]
    fn a_compact_size_smaller_than_the_fields_it_is_stored_in_is_rejected() {
        assert_eq!(CompactSize::new(7), None);
    }

    #[test]
    fn a_compact_size_of_exactly_the_size_and_type_fields_is_accepted() {
        assert_eq!(CompactSize::new(8).map(CompactSize::get), Some(8));
    }

    #[test]
    fn an_extended_size_smaller_than_the_fields_it_is_stored_in_is_rejected() {
        assert_eq!(ExtendedSize::new(15), None);
    }

    #[test]
    fn an_extended_size_of_exactly_the_size_type_and_largesize_fields_is_accepted() {
        assert_eq!(ExtendedSize::new(16).map(ExtendedSize::get), Some(16));
    }

    #[test]
    fn the_compact_and_extended_forms_report_the_same_total() {
        let compact = BoxSize::Compact(CompactSize::new(4096).unwrap());
        let extended = BoxSize::Extended(ExtendedSize::new(4096).unwrap());

        assert_eq!(compact.total_bytes(), extended.total_bytes());
    }

    #[test]
    fn the_compact_and_extended_forms_stay_distinct_values() {
        let compact = BoxSize::Compact(CompactSize::new(4096).unwrap());
        let extended = BoxSize::Extended(ExtendedSize::new(4096).unwrap());

        assert_ne!(compact, extended);
    }

    #[test]
    fn the_end_of_file_form_declares_no_total() {
        assert_eq!(BoxSize::ToEndOfFile.total_bytes(), None);
    }
}
