//! [`Error`], the reason a box does not read off bytes or write into them

use core::error;
use core::fmt;

use crate::codec::field::FieldWidth;
use crate::data_types::fourcc::FourCC;
use crate::framing::box_type::BoxType;

mod kind;

pub use kind::{Category, ErrorKind};

/// Boxes a failure holds of the path out of the containers it was read in
const CONTAINER_DEPTH: usize = 8;

/// Reason a box does not read off bytes, or does not write into them
///
/// What went wrong is one [`kind`](Self::kind), which carries the values that
/// describe it, and what a caller does about it is one
/// [`category`](Self::category).
///
/// A box read inside a container names the boxes it was reached through, as
/// [`containers`](Self::containers). Each container adds itself as the failure
/// passes out through it, so the path reads from the outermost box down to the
/// one that went wrong.
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxType, Category, Error, ErrorKind, FourCC};
///
/// // A container names itself as a child failure passes out through it
/// let failure = Error::unsupported_version(2)
///     .in_container(BoxType::compact(*b"tkhd"))
///     .in_container(BoxType::compact(*b"trak"));
///
/// // What went wrong, with its values, and what a caller does about it
/// assert!(matches!(
///     failure.kind(),
///     ErrorKind::UnsupportedVersion { version: 2, .. }
/// ));
/// assert_eq!(failure.category(), Category::Unsupported);
///
/// // Where it went wrong, outermost box first
/// assert_eq!(
///     failure.containers().collect::<Vec<_>>(),
///     [FourCC::new(*b"trak"), FourCC::new(*b"tkhd")]
/// );
///
/// // A reader of the failure sees the path before the reason
/// assert_eq!(
///     failure.to_string(),
///     "in trak/tkhd: full box declares version 2, which this box does not read"
/// );
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Error {
    kind: ErrorKind,
    containers: [Option<FourCC>; CONTAINER_DEPTH],
    dropped_containers: bool,
}

impl Error {
    /// Returns a failure of `kind`
    const fn new(kind: ErrorKind) -> Self {
        Self {
            kind,
            containers: [None; CONTAINER_DEPTH],
            dropped_containers: false,
        }
    }

    /// Returns the failure of an input that ends inside the header of a box
    #[must_use]
    pub const fn truncated_header(needed_bytes: u64, available_bytes: u64) -> Self {
        Self::new(ErrorKind::TruncatedHeader {
            needed_bytes,
            available_bytes,
        })
    }

    /// Returns the failure of a box declaring a total below the header it prefixes
    #[must_use]
    pub const fn size_below_header(header_bytes: u64, declared_bytes: u64) -> Self {
        Self::new(ErrorKind::SizeBelowHeader {
            header_bytes,
            declared_bytes,
        })
    }

    /// Returns the failure of a box whose declared total overruns the input
    #[must_use]
    pub const fn truncated_box(needed_bytes: u64, available_bytes: u64) -> Self {
        Self::new(ErrorKind::TruncatedBox {
            needed_bytes,
            available_bytes,
        })
    }

    /// Returns the failure of a box read as a type the input does not hold there
    #[must_use]
    pub const fn box_type_mismatch(expected_box_type: BoxType, found_box_type: BoxType) -> Self {
        Self::new(ErrorKind::BoxTypeMismatch {
            expected_box_type,
            found_box_type,
        })
    }

    /// Returns the failure of a payload that ends inside a field
    #[must_use]
    pub const fn truncated_payload(needed_bytes: u64, available_bytes: u64) -> Self {
        Self::new(ErrorKind::TruncatedPayload {
            needed_bytes,
            available_bytes,
        })
    }

    /// Returns the failure of a payload holding bytes past the fields it reads
    #[must_use]
    pub const fn trailing_payload(needed_bytes: u64, available_bytes: u64) -> Self {
        Self::new(ErrorKind::TrailingPayload {
            needed_bytes,
            available_bytes,
        })
    }

    /// Returns the failure of a buffer that ends inside what is written into it
    #[must_use]
    pub const fn truncated_buffer(needed_bytes: u64, available_bytes: u64) -> Self {
        Self::new(ErrorKind::TruncatedBuffer {
            needed_bytes,
            available_bytes,
        })
    }

    /// Returns the failure of a buffer holding bytes past the fields a box wrote
    #[must_use]
    pub const fn trailing_buffer(needed_bytes: u64, available_bytes: u64) -> Self {
        Self::new(ErrorKind::TrailingBuffer {
            needed_bytes,
            available_bytes,
        })
    }

    /// Returns the failure of a buffer that is not the length a payload declared
    #[must_use]
    pub const fn buffer_length_mismatch(declared_bytes: u64, offered_bytes: u64) -> Self {
        Self::new(ErrorKind::BufferLengthMismatch {
            declared_bytes,
            offered_bytes,
        })
    }

    /// Returns the failure of a value wider than the field it was given to
    #[must_use]
    pub const fn out_of_range(value: u64, width: FieldWidth) -> Self {
        let field_bytes = match width {
            FieldWidth::Compact => 4,
            FieldWidth::Extended => 8,
        };

        Self::new(ErrorKind::OutOfRange { value, field_bytes })
    }

    /// Returns the failure of a full box declaring flags the spec forbids together
    #[must_use]
    pub const fn conflicting_flags(flags: u32) -> Self {
        Self::new(ErrorKind::ConflictingFlags { flags })
    }

    /// Returns the failure of a field the spec counts from 1 holding 0
    #[must_use]
    pub const fn zero_index() -> Self {
        Self::new(ErrorKind::ZeroIndex)
    }

    /// Returns the failure of a container lacking a child the spec marks mandatory
    #[must_use]
    pub const fn missing_mandatory_box(box_type: BoxType) -> Self {
        Self::new(ErrorKind::MissingMandatoryBox { box_type })
    }

    /// Returns the failure of a container holding more of a child than it may
    #[must_use]
    pub const fn duplicate_box(box_type: BoxType) -> Self {
        Self::new(ErrorKind::DuplicateBox { box_type })
    }

    /// Returns the failure of a container holding a child a field of it forbids
    #[must_use]
    pub const fn forbidden_child_box(box_type: BoxType) -> Self {
        Self::new(ErrorKind::ForbiddenChildBox { box_type })
    }

    /// Returns the failure of a container holding none of the boxes it must hold one of
    #[must_use]
    pub const fn missing_alternative_box(alternatives: &'static [BoxType]) -> Self {
        Self::new(ErrorKind::MissingAlternativeBox { alternatives })
    }

    /// Returns the failure of a container holding more than one of the boxes it may hold one of
    #[must_use]
    pub const fn duplicate_alternative_box(alternatives: &'static [BoxType]) -> Self {
        Self::new(ErrorKind::DuplicateAlternativeBox { alternatives })
    }

    /// Returns the failure of a count that disagrees with the entries it frames
    #[must_use]
    pub const fn entry_count_mismatch(declared_entries: u64, actual_entries: u64) -> Self {
        Self::new(ErrorKind::EntryCountMismatch {
            declared_entries,
            actual_entries,
        })
    }

    /// Returns the failure of a container holding a box this implementation does not read
    #[must_use]
    pub const fn unsupported_box(box_type: BoxType) -> Self {
        Self::new(ErrorKind::UnsupportedBox { box_type })
    }

    /// Returns the failure of a full box declaring a version the box does not read
    #[must_use]
    pub const fn unsupported_version(version: u8) -> Self {
        Self::new(ErrorKind::UnsupportedVersion { version })
    }

    /// Returns the failure of a box declaring a field size the box does not read
    #[must_use]
    pub const fn unsupported_field_size(field_size: u8) -> Self {
        Self::new(ErrorKind::UnsupportedFieldSize { field_size })
    }

    /// Returns the failure of a full box declaring flags the box does not read
    #[must_use]
    pub const fn unsupported_flags(flags: u32) -> Self {
        Self::new(ErrorKind::UnsupportedFlags { flags })
    }

    /// Returns the failure of a box stating a value the box does not read in one of its fields
    #[must_use]
    pub const fn unsupported_value() -> Self {
        Self::new(ErrorKind::UnsupportedValue)
    }

    /// Returns the failure with `container` added to the boxes it was reached through
    ///
    /// A container calls this as a child failure passes through it, which builds
    /// the path a reader of the failure walks. A path longer than a failure
    /// holds keeps its innermost boxes, the ones nearest what went wrong.
    #[must_use]
    pub fn in_container(mut self, container: BoxType) -> Self {
        match self.containers.iter_mut().find(|slot| slot.is_none()) {
            Some(slot) => *slot = Some(container.four_cc()),
            None => self.dropped_containers = true,
        }

        self
    }

    /// Returns what went wrong, with the values that describe it
    #[must_use]
    pub const fn kind(self) -> ErrorKind {
        self.kind
    }

    /// Returns what a caller does about the failure
    #[must_use]
    pub const fn category(self) -> Category {
        self.kind.category()
    }

    /// Returns the boxes the failure was reached through, outermost first
    pub fn containers(self) -> impl Iterator<Item = FourCC> {
        self.containers.into_iter().rev().flatten()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut containers = self.containers();
        if let Some(outermost) = containers.next() {
            let opening = if self.dropped_containers {
                "in .../"
            } else {
                "in "
            };
            formatter.write_str(opening)?;
            write!(formatter, "{outermost}")?;
            for container in containers {
                write!(formatter, "/{container}")?;
            }
            formatter.write_str(": ")?;
        }

        match self.kind {
            ErrorKind::TruncatedHeader {
                needed_bytes,
                available_bytes,
            } => write!(
                formatter,
                "box header of {needed_bytes} bytes cut short by an input of {available_bytes}"
            ),
            ErrorKind::SizeBelowHeader {
                header_bytes,
                declared_bytes,
            } => write!(
                formatter,
                "box declares a total of {declared_bytes} bytes, below its {header_bytes}-byte \
                 header"
            ),
            ErrorKind::TruncatedBox {
                needed_bytes,
                available_bytes,
            } => write!(
                formatter,
                "box of {needed_bytes} bytes cut short by an input of {available_bytes}"
            ),
            ErrorKind::BoxTypeMismatch {
                expected_box_type,
                found_box_type,
            } => write!(
                formatter,
                "input holds a {found_box_type} box where a {expected_box_type} box was expected"
            ),
            ErrorKind::TruncatedPayload {
                needed_bytes,
                available_bytes,
            } => write!(
                formatter,
                "box payload of {needed_bytes} bytes cut short by an input of {available_bytes}"
            ),
            ErrorKind::TrailingPayload {
                needed_bytes,
                available_bytes,
            } => write!(
                formatter,
                "box payload leaves {} bytes past the fields it holds",
                available_bytes.saturating_sub(needed_bytes)
            ),
            ErrorKind::TruncatedBuffer {
                needed_bytes,
                available_bytes,
            } => write!(
                formatter,
                "value of {needed_bytes} bytes needs a buffer at least that long, not \
                 {available_bytes}"
            ),
            ErrorKind::TrailingBuffer {
                needed_bytes,
                available_bytes,
            } => write!(
                formatter,
                "buffer holds {} bytes past the fields the box wrote",
                available_bytes.saturating_sub(needed_bytes)
            ),
            ErrorKind::BufferLengthMismatch {
                declared_bytes,
                offered_bytes,
            } => write!(
                formatter,
                "box payload of {declared_bytes} bytes needs a buffer of that length, not \
                 {offered_bytes}"
            ),
            ErrorKind::OutOfRange { value, field_bytes } => write!(
                formatter,
                "value {value} does not fit the {field_bytes} bytes of the field it was given to"
            ),
            ErrorKind::ConflictingFlags { flags } => write!(
                formatter,
                "full box declares flags {flags:#08x}, which the spec does not allow together"
            ),
            ErrorKind::ZeroIndex => {
                formatter.write_str("box holds 0 in a field the spec counts from 1")
            }
            ErrorKind::MissingMandatoryBox { box_type } => {
                write!(formatter, "container holds no mandatory {box_type} box")
            }
            ErrorKind::DuplicateBox { box_type } => write!(
                formatter,
                "container holds more than one {box_type} box, which may appear once"
            ),
            ErrorKind::ForbiddenChildBox { box_type } => write!(
                formatter,
                "container holds a {box_type} box that a field of it forbids"
            ),
            ErrorKind::MissingAlternativeBox { alternatives } => write!(
                formatter,
                "container holds none of the {} boxes, one of which it must hold",
                Listed(alternatives)
            ),
            ErrorKind::DuplicateAlternativeBox { alternatives } => write!(
                formatter,
                "container holds more than one of the {} boxes, of which one may appear",
                Listed(alternatives)
            ),
            ErrorKind::UnsupportedBox { box_type } => write!(
                formatter,
                "container holds a {box_type} box, which this implementation does not read"
            ),
            ErrorKind::EntryCountMismatch {
                declared_entries,
                actual_entries,
            } => write!(
                formatter,
                "box declares {declared_entries} entries but holds {actual_entries}"
            ),
            ErrorKind::UnsupportedVersion { version } => write!(
                formatter,
                "full box declares version {version}, which this box does not read"
            ),
            ErrorKind::UnsupportedFieldSize { field_size } => write!(
                formatter,
                "box declares entries {field_size} bits wide, which this box does not read"
            ),
            ErrorKind::UnsupportedFlags { flags } => write!(
                formatter,
                "full box declares flags {flags:#08x}, which this box does not read"
            ),
            ErrorKind::UnsupportedValue => formatter
                .write_str("box states a value this box does not read in one of its fields"),
        }
    }
}

impl error::Error for Error {}

/// Failure that names each box it passes out through, keeping its type
///
/// A box read inside a container fails inside that container as well, and the
/// container names itself on the failure as it passes out. `ChildBoxes`
/// requires this of the [`Error`](crate::BoxDecode::Error) of the child it
/// reads.
///
/// A failure with no path to add to returns itself unchanged.
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxType, Error, InContainer};
///
/// // A failure of a vendor crate, which either names a box or does not
/// #[derive(Clone, Copy, PartialEq, Debug)]
/// enum VendorError {
///     Box(Error),
///     Bitstream,
/// }
///
/// impl InContainer for VendorError {
///     fn in_container(self, container: BoxType) -> Self {
///         match self {
///             Self::Box(box_error) => Self::Box(box_error.in_container(container)),
///             Self::Bitstream => Self::Bitstream,
///         }
///     }
/// }
///
/// // A failure of a box takes the container on its path
/// let failure = VendorError::Box(Error::unsupported_version(2))
///     .in_container(BoxType::compact(*b"vndr"));
/// assert_eq!(
///     failure,
///     VendorError::Box(Error::unsupported_version(2).in_container(BoxType::compact(*b"vndr")))
/// );
///
/// // A failure with no path stays as it was
/// assert_eq!(
///     VendorError::Bitstream.in_container(BoxType::compact(*b"vndr")),
///     VendorError::Bitstream
/// );
/// ```
pub trait InContainer {
    /// Returns the failure with `container` added to the boxes it was reached through
    #[must_use]
    fn in_container(self, container: BoxType) -> Self;
}

impl InContainer for Error {
    fn in_container(self, container: BoxType) -> Self {
        Self::in_container(self, container)
    }
}

/// Box types a failure names one of, as `Display` lists them
struct Listed(&'static [BoxType]);

impl fmt::Display for Listed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some((first, rest)) = self.0.split_first() else {
            return Ok(());
        };

        write!(formatter, "{first}")?;
        for box_type in rest {
            write!(formatter, ", {box_type}")?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString as _;
    use alloc::vec;
    use alloc::vec::Vec;

    use super::{Error, ErrorKind};
    use crate::codec::field::FieldWidth;
    use crate::data_types::fourcc::FourCC;
    use crate::framing::box_type::BoxType;

    /// Box types the sample table of ISO/IEC 14496-12 §8.7.3.1 states its sample sizes with
    pub(super) const SAMPLE_SIZE_BOXES: &[BoxType] =
        &[BoxType::compact(*b"stsz"), BoxType::compact(*b"stz2")];

    /// Box types the media information box of ISO/IEC 14496-12 §8.4.5 takes its media
    /// header from
    const MEDIA_HEADER_BOXES: &[BoxType] = &[
        BoxType::compact(*b"vmhd"),
        BoxType::compact(*b"smhd"),
        BoxType::compact(*b"hmhd"),
        BoxType::compact(*b"nmhd"),
        BoxType::compact(*b"sthd"),
    ];

    #[test]
    fn a_failure_names_the_boxes_it_was_reached_through_outermost_first() {
        let error = Error::truncated_payload(20, 12)
            .in_container(BoxType::compact(*b"tkhd"))
            .in_container(BoxType::compact(*b"trak"));

        assert_eq!(
            error.to_string(),
            "in trak/tkhd: box payload of 20 bytes cut short by an input of 12"
        );
    }

    #[test]
    fn a_path_longer_than_a_failure_holds_keeps_the_boxes_nearest_the_failure() {
        let containers = [
            *b"moov", *b"trak", *b"mdia", *b"minf", *b"stbl", *b"stsd", *b"avc1", *b"btrt",
            *b"free",
        ];
        let error = containers
            .iter()
            .rev()
            .fold(Error::truncated_payload(20, 12), |error, container| {
                error.in_container(BoxType::compact(*container))
            });

        assert_eq!(
            error.to_string(),
            "in .../trak/mdia/minf/stbl/stsd/avc1/btrt/free: \
             box payload of 20 bytes cut short by an input of 12"
        );
    }

    #[test]
    fn a_value_out_of_range_names_the_bytes_of_the_field_it_was_given_to() {
        assert_eq!(
            Error::out_of_range(0x1_0000_0000, FieldWidth::Compact).kind(),
            ErrorKind::OutOfRange {
                value: 0x1_0000_0000,
                field_bytes: 4,
            }
        );
    }

    #[test]
    fn the_containers_a_failure_was_reached_through_read_outermost_first() {
        let error = Error::truncated_payload(20, 12)
            .in_container(BoxType::compact(*b"tkhd"))
            .in_container(BoxType::compact(*b"trak"))
            .in_container(BoxType::compact(*b"moov"));

        assert_eq!(
            error.containers().collect::<Vec<_>>(),
            vec![
                FourCC::new(*b"moov"),
                FourCC::new(*b"trak"),
                FourCC::new(*b"tkhd"),
            ]
        );
    }

    #[test]
    fn display_of_a_failure_that_names_a_box_names_its_type() {
        assert_eq!(
            Error::missing_mandatory_box(BoxType::compact(*b"mvhd")).to_string(),
            "container holds no mandatory mvhd box"
        );
        assert_eq!(
            Error::duplicate_box(BoxType::compact(*b"tkhd")).to_string(),
            "container holds more than one tkhd box, which may appear once"
        );
        assert_eq!(
            Error::forbidden_child_box(BoxType::compact(*b"trun")).to_string(),
            "container holds a trun box that a field of it forbids"
        );
        assert_eq!(
            Error::box_type_mismatch(BoxType::compact(*b"moov"), BoxType::compact(*b"moof"))
                .to_string(),
            "input holds a moof box where a moov box was expected"
        );
        assert_eq!(
            Error::unsupported_box(BoxType::compact(*b"sgpd")).to_string(),
            "container holds a sgpd box, which this implementation does not read"
        );
    }

    #[test]
    fn display_of_a_failure_about_a_slot_several_box_types_fill_names_them_all() {
        assert_eq!(
            Error::missing_alternative_box(SAMPLE_SIZE_BOXES).to_string(),
            "container holds none of the stsz, stz2 boxes, one of which it must hold"
        );
        assert_eq!(
            Error::duplicate_alternative_box(MEDIA_HEADER_BOXES).to_string(),
            "container holds more than one of the vmhd, smhd, hmhd, nmhd, sthd boxes, \
             of which one may appear"
        );
    }

    #[test]
    fn display_of_a_failure_a_full_box_declared_names_what_it_declared() {
        assert_eq!(
            Error::unsupported_version(2).to_string(),
            "full box declares version 2, which this box does not read"
        );
        assert_eq!(
            Error::unsupported_flags(0x0000_1000).to_string(),
            "full box declares flags 0x001000, which this box does not read"
        );
        assert_eq!(
            Error::conflicting_flags(0x0000_0404).to_string(),
            "full box declares flags 0x000404, which the spec does not allow together"
        );
        assert_eq!(
            Error::unsupported_field_size(12).to_string(),
            "box declares entries 12 bits wide, which this box does not read"
        );
        assert_eq!(
            Error::zero_index().to_string(),
            "box holds 0 in a field the spec counts from 1"
        );
        assert_eq!(
            Error::unsupported_value().to_string(),
            "box states a value this box does not read in one of its fields"
        );
    }

    #[test]
    fn display_of_a_failure_that_counts_entries_names_both_counts() {
        assert_eq!(
            Error::entry_count_mismatch(4, 2).to_string(),
            "box declares 4 entries but holds 2"
        );
    }

    #[test]
    fn display_of_a_failure_that_counts_bytes_names_both_lengths() {
        assert_eq!(
            Error::truncated_header(16, 12).to_string(),
            "box header of 16 bytes cut short by an input of 12"
        );
        assert_eq!(
            Error::size_below_header(24, 20).to_string(),
            "box declares a total of 20 bytes, below its 24-byte header"
        );
        assert_eq!(
            Error::truncated_box(32, 24).to_string(),
            "box of 32 bytes cut short by an input of 24"
        );
        assert_eq!(
            Error::trailing_payload(12, 16).to_string(),
            "box payload leaves 4 bytes past the fields it holds"
        );
        assert_eq!(
            Error::truncated_buffer(16, 12).to_string(),
            "value of 16 bytes needs a buffer at least that long, not 12"
        );
        assert_eq!(
            Error::trailing_buffer(12, 16).to_string(),
            "buffer holds 4 bytes past the fields the box wrote"
        );
        assert_eq!(
            Error::buffer_length_mismatch(4, 8).to_string(),
            "box payload of 4 bytes needs a buffer of that length, not 8"
        );
        assert_eq!(
            Error::out_of_range(0x1_0000_0000, FieldWidth::Compact).to_string(),
            "value 4294967296 does not fit the 4 bytes of the field it was given to"
        );
    }
}
