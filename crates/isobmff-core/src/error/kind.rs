//! [`ErrorKind`], what a failure of reading or writing a box is, and [`Category`]

use crate::framing::box_type::BoxType;

/// What a failure of reading or writing a box is, with the values it carries
///
/// The vocabulary is this crate's own: reading one box off a slice and writing
/// one into a buffer name their failures here. A layer above holds kinds of its
/// own for the failures it detects, and carries these through whole rather than
/// translating them. Each kind carries the values that describe it, and falls
/// in one [`Category`], which [`Error::category`](crate::Error::category)
/// reports.
///
/// The situations a box reaches are added to as ISO/IEC 14496-12 is read
/// further, so a match on this must leave room for kinds that are not here yet,
/// and a match on a kind must leave room for values that are not here yet.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ErrorKind {
    /// Input ends inside the header of a box
    #[non_exhaustive]
    TruncatedHeader {
        /// Length the header reaches
        needed_bytes: u64,
        /// Length the input offered
        available_bytes: u64,
    },
    /// Total a box declares is smaller than the header it prefixes
    #[non_exhaustive]
    SizeBelowHeader {
        /// Length the header occupies
        needed_bytes: u64,
        /// Total the `size` or `largesize` field declares
        available_bytes: u64,
    },
    /// Total a box declares overruns the input
    #[non_exhaustive]
    TruncatedBox {
        /// Length the box occupies
        needed_bytes: u64,
        /// Length the input offered
        available_bytes: u64,
    },
    /// Box read as one type is of another
    #[non_exhaustive]
    BoxTypeMismatch {
        /// Type the box was read as
        box_type: BoxType,
        /// Type the input holds
        found_box_type: BoxType,
    },
    /// Payload of a box ends inside a field
    #[non_exhaustive]
    TruncatedPayload {
        /// Length the fields read so far require
        needed_bytes: u64,
        /// Length the payload offered
        available_bytes: u64,
    },
    /// Payload of a box holds bytes past the fields it reads
    #[non_exhaustive]
    TrailingPayload {
        /// Length the fields took
        needed_bytes: u64,
        /// Length the payload holds
        available_bytes: u64,
    },
    /// Full box declares flags the spec does not allow together
    #[non_exhaustive]
    ConflictingFlags {
        /// Flags the box declares
        flags: u32,
    },
    /// Field the spec counts from 1 holds 0
    #[non_exhaustive]
    ZeroIndex,
    /// Container lacks a child box the spec marks mandatory
    #[non_exhaustive]
    MissingMandatoryBox {
        /// Type of the child that is missing
        box_type: BoxType,
    },
    /// Container holds more of a child box than its quantity allows
    #[non_exhaustive]
    DuplicateBox {
        /// Type of the child held more than once
        box_type: BoxType,
    },
    /// Container holds a child box that a field of it forbids
    #[non_exhaustive]
    ForbiddenChildBox {
        /// Type of the child that is forbidden
        box_type: BoxType,
    },
    /// Container holds none of the boxes the spec has it hold exactly one of
    #[non_exhaustive]
    MissingAlternativeBox {
        /// Box types the container must hold one of
        alternatives: &'static [BoxType],
    },
    /// Container holds more than one of the boxes the spec has it hold exactly one of
    #[non_exhaustive]
    DuplicateAlternativeBox {
        /// Box types the container may hold one of
        alternatives: &'static [BoxType],
    },
    /// Count a box declares does not match the entries it frames for itself
    #[non_exhaustive]
    EntryCountMismatch {
        /// Count the `entry_count` field declares
        needed_entries: u64,
        /// Count the payload holds
        available_entries: u64,
    },
    /// Container holds a box this implementation does not read
    #[non_exhaustive]
    UnsupportedBox {
        /// Type of the child that is not read
        box_type: BoxType,
    },
    /// Full box declares a version the box does not read
    #[non_exhaustive]
    UnsupportedVersion {
        /// Version the box declares
        version: u8,
    },
    /// Full box declares flags the box does not read
    #[non_exhaustive]
    UnsupportedFlags {
        /// Flags the box declares
        flags: u32,
    },
    /// Box declares its entries in a width the box does not read
    #[non_exhaustive]
    UnsupportedFieldSize {
        /// Width in bits the box declares
        field_size: u8,
    },
    /// Box states a value the box does not read in one of its fields
    #[non_exhaustive]
    UnsupportedValue,
    /// Buffer ends inside the value being written into it
    #[non_exhaustive]
    TruncatedBuffer {
        /// Length the value requires
        needed_bytes: u64,
        /// Length the buffer offered
        available_bytes: u64,
    },
    /// Buffer holds bytes past the fields a box wrote
    #[non_exhaustive]
    TrailingBuffer {
        /// Length the fields wrote
        needed_bytes: u64,
        /// Length the buffer holds
        available_bytes: u64,
    },
    /// Buffer offered for a payload is not the length the payload declared
    #[non_exhaustive]
    BufferLengthMismatch {
        /// Length the payload declares
        needed_bytes: u64,
        /// Length the buffer offered
        available_bytes: u64,
    },
    /// Value is wider than the field it was given to
    #[non_exhaustive]
    OutOfRange {
        /// Value the field was given
        value: u64,
        /// Width in bytes of the field
        needed_bytes: u64,
    },
}

impl ErrorKind {
    /// Returns what a caller does about a failure of this kind
    pub(crate) const fn category(self) -> Category {
        match self {
            Self::TruncatedHeader { .. }
            | Self::SizeBelowHeader { .. }
            | Self::TruncatedBox { .. }
            | Self::BoxTypeMismatch { .. }
            | Self::TruncatedPayload { .. }
            | Self::TrailingPayload { .. }
            | Self::ConflictingFlags { .. }
            | Self::ZeroIndex
            | Self::MissingMandatoryBox { .. }
            | Self::DuplicateBox { .. }
            | Self::ForbiddenChildBox { .. }
            | Self::MissingAlternativeBox { .. }
            | Self::DuplicateAlternativeBox { .. }
            | Self::EntryCountMismatch { .. } => Category::Malformed,
            Self::UnsupportedBox { .. }
            | Self::UnsupportedVersion { .. }
            | Self::UnsupportedFlags { .. }
            | Self::UnsupportedFieldSize { .. }
            | Self::UnsupportedValue => Category::Unsupported,
            Self::TruncatedBuffer { .. }
            | Self::TrailingBuffer { .. }
            | Self::BufferLengthMismatch { .. }
            | Self::OutOfRange { .. } => Category::Usage,
        }
    }
}

/// What a caller does about a failure
///
/// A kind names one situation and there are many of them; this names what the
/// situations have in common for whoever has to act on one. The three ask for
/// three different things: a file that cannot be read, a file this
/// implementation does not read, and a call that should not have been made.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Category {
    /// Boxes do not form what the format requires, and the file cannot stand as it is
    ///
    /// The file being read is malformed, or what is being written would lay
    /// down one that is.
    Malformed,
    /// Format allows what the file holds, and this implementation does not read it
    ///
    /// The file is not at fault, so a caller may leave the box unread and carry
    /// on with the ones it does read.
    Unsupported,
    /// Call was made with something the API does not take, or in an order it does not
    ///
    /// Nothing about the file is wrong; the code that made the call is. Writing
    /// a value reports this as well: the buffer it is handed is the caller's to
    /// size, and a value too wide for its field was built before it was written.
    Usage,
}

#[cfg(test)]
mod tests {
    use super::Category;
    use crate::error::Error;
    use crate::error::tests::SAMPLE_SIZE_BOXES;
    use crate::framing::box_type::BoxType;

    #[test]
    fn a_kind_falls_in_the_category_its_situation_asks_for() {
        assert_eq!(Error::truncated_box(32, 24).category(), Category::Malformed);
        assert_eq!(
            Error::unsupported_version(2).category(),
            Category::Unsupported
        );
        assert_eq!(
            Error::unsupported_box(BoxType::compact(*b"sgpd")).category(),
            Category::Unsupported
        );
        assert_eq!(
            Error::unsupported_field_size(12).category(),
            Category::Unsupported
        );
        assert_eq!(
            Error::missing_alternative_box(SAMPLE_SIZE_BOXES).category(),
            Category::Malformed
        );
        assert_eq!(
            Error::buffer_length_mismatch(4, 8).category(),
            Category::Usage
        );
        assert_eq!(Error::zero_index().category(), Category::Malformed);
        assert_eq!(Error::unsupported_value().category(), Category::Unsupported);
    }
}
