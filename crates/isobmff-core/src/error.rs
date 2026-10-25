//! [`Error`], the reason a box does not read off bytes or write into them, and the [`ErrorKind`] and [`Category`] it carries

use core::error;
use core::fmt;

use crate::codec::field::FieldWidth;
use crate::data_types::fourcc::FourCc;
use crate::framing::box_type::BoxType;

/// Boxes a failure holds of the path out of the containers it was read in
const CONTAINER_DEPTH: usize = 8;

/// Reason a box does not read off bytes, or does not write into them
///
/// Every box reads and writes with this one type, whichever crate defines the
/// box. What went wrong is one [`kind`](Self::kind), which carries the values
/// that describe it, and what a caller does about it is one
/// [`category`](Self::category).
///
/// A box read inside a container names the boxes it was reached through, as
/// [`containers`](Self::containers). Each container adds itself with
/// [`in_container`](Self::in_container) as the failure passes out through it,
/// so the path reads from the outermost box down to the one that went wrong.
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxType, Category, Error, ErrorKind, FourCc};
///
/// // A vendor box fails on a version it does not read, inside a vendor container
/// let failure = Error::unsupported_version(2)
///     .in_container(BoxType::compact(*b"vndb"))
///     .in_container(BoxType::compact(*b"vndc"));
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
///     [FourCc::new(*b"vndc"), FourCc::new(*b"vndb")]
/// );
///
/// // A reader of the failure sees the path before the reason
/// assert_eq!(
///     failure.to_string(),
///     "in vndc/vndb: full box declares version 2, which this box does not read"
/// );
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Error {
    kind: ErrorKind,
    containers: [Option<FourCc>; CONTAINER_DEPTH],
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

    /// Returns the failure of a field holding a value the specification does not allow there
    ///
    /// `field` is the name of the field as the specification spells it. A
    /// failure another kind names is reported as that kind instead.
    #[must_use]
    pub const fn invalid_value(field: &'static str) -> Self {
        Self::new(ErrorKind::InvalidValue { field })
    }

    /// Returns the failure of a `utf8string` field holding bytes that are not UTF-8
    #[must_use]
    pub const fn invalid_utf8_string() -> Self {
        Self::new(ErrorKind::InvalidUtf8String)
    }

    /// Returns the failure of a count that disagrees with the entries it frames
    #[must_use]
    pub const fn entry_count_mismatch(declared_entries: u64, actual_entries: u64) -> Self {
        Self::new(ErrorKind::EntryCountMismatch {
            declared_entries,
            actual_entries,
        })
    }

    /// Returns the failure of a full box declaring a version the box does not read
    #[must_use]
    pub const fn unsupported_version(version: u8) -> Self {
        Self::new(ErrorKind::UnsupportedVersion { version })
    }

    /// Returns the failure of a full box setting a flag that changes its syntax in a way it does not read
    ///
    /// A flag the box does not recognize is not this failure: ISO/IEC
    /// 14496-12 §4.2.2 has a reader that supports the version read on past
    /// it.
    #[must_use]
    pub const fn unsupported_flags(flags: u32) -> Self {
        Self::new(ErrorKind::UnsupportedFlags { flags })
    }

    /// Returns the failure of a field holding a value the specification allows and the box does not read
    ///
    /// `field` is the name of the field as the specification spells it.
    #[must_use]
    pub const fn unsupported_value(field: &'static str) -> Self {
        Self::new(ErrorKind::UnsupportedValue { field })
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
    pub fn containers(self) -> impl Iterator<Item = FourCc> {
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
            write!(formatter, "{opening}{outermost}")?;
            for container in containers {
                write!(formatter, "/{container}")?;
            }
            formatter.write_str(": ")?;
        }

        fmt::Display::fmt(&self.kind, formatter)
    }
}

impl error::Error for Error {}

/// What a failure of reading or writing a box is, with the values it carries
///
/// The vocabulary is shared by every box: this crate's own reading and writing
/// name their failures here, and so does a box that a crate depending on this
/// one defines. A layer above holds kinds of its own for the failures it
/// detects, and carries these through whole rather than translating them. Each
/// kind carries the values that describe it, and falls in one [`Category`],
/// which [`Error::category`](crate::Error::category) reports.
///
/// A box reports the kind that names its failure. [`InvalidValue`](Self::InvalidValue)
/// and [`UnsupportedValue`](Self::UnsupportedValue) are for a failure no other
/// kind names, and say which field of the box it lies in.
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
        /// Length of the header
        header_bytes: u64,
        /// Total the `size` or `largesize` field declares
        declared_bytes: u64,
    },
    /// Total a box declares overruns the input
    #[non_exhaustive]
    TruncatedBox {
        /// Total the box declares
        needed_bytes: u64,
        /// Length the input offered
        available_bytes: u64,
    },
    /// Box read as one type lies in the input as another
    #[non_exhaustive]
    BoxTypeMismatch {
        /// Type the box was read as
        expected_box_type: BoxType,
        /// Type the input holds there
        found_box_type: BoxType,
    },
    /// Payload ends inside a field
    #[non_exhaustive]
    TruncatedPayload {
        /// Length the fields reach
        needed_bytes: u64,
        /// Length the payload offered
        available_bytes: u64,
    },
    /// Field holds a value the specification does not allow there
    #[non_exhaustive]
    InvalidValue {
        /// Name of the field as the specification spells it
        field: &'static str,
    },
    /// Field of the basic type `utf8string` holds bytes that are not UTF-8
    ///
    /// ISO/IEC 14496-12 §4.2.1 defines the type as UTF-8. A reader keeps such
    /// bytes as found, so this is reported where they would be written.
    InvalidUtf8String,
    /// Count a box declares does not match the entries it frames for itself
    #[non_exhaustive]
    EntryCountMismatch {
        /// Count the `entry_count` field declares
        declared_entries: u64,
        /// Count the payload holds
        actual_entries: u64,
    },
    /// Full box declares a version the box does not read
    #[non_exhaustive]
    UnsupportedVersion {
        /// Version the box declares
        version: u8,
    },
    /// Full box sets a flag that changes the syntax of the box in a way it does not read
    #[non_exhaustive]
    UnsupportedFlags {
        /// Flags the box declares
        flags: u32,
    },
    /// Field holds a value the specification allows and the box does not read
    #[non_exhaustive]
    UnsupportedValue {
        /// Name of the field as the specification spells it
        field: &'static str,
    },
    /// Buffer ends inside the value being written into it
    #[non_exhaustive]
    TruncatedBuffer {
        /// Length the value requires
        needed_bytes: u64,
        /// Length the buffer offered
        available_bytes: u64,
    },
    /// Buffer holds bytes past the fields a box wrote into it
    #[non_exhaustive]
    TrailingBuffer {
        /// Length the fields claimed
        needed_bytes: u64,
        /// Length the buffer offered
        available_bytes: u64,
    },
    /// Buffer is not the length the payload declared
    #[non_exhaustive]
    BufferLengthMismatch {
        /// Length the payload declared
        declared_bytes: u64,
        /// Length the buffer offered
        offered_bytes: u64,
    },
    /// Value is wider than the field it was given to
    #[non_exhaustive]
    OutOfRange {
        /// Value given
        value: u64,
        /// Width in bytes of the field
        field_bytes: u64,
    },
}

impl ErrorKind {
    /// Returns what a caller does about a failure of this kind
    #[must_use]
    pub const fn category(self) -> Category {
        match self {
            Self::TruncatedHeader { .. }
            | Self::SizeBelowHeader { .. }
            | Self::TruncatedBox { .. }
            | Self::BoxTypeMismatch { .. }
            | Self::TruncatedPayload { .. }
            | Self::InvalidValue { .. }
            | Self::InvalidUtf8String
            | Self::EntryCountMismatch { .. } => Category::Malformed,
            Self::UnsupportedVersion { .. }
            | Self::UnsupportedFlags { .. }
            | Self::UnsupportedValue { .. } => Category::Unsupported,
            Self::TruncatedBuffer { .. }
            | Self::TrailingBuffer { .. }
            | Self::BufferLengthMismatch { .. }
            | Self::OutOfRange { .. } => Category::Usage,
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::TruncatedHeader {
                needed_bytes,
                available_bytes,
            } => write!(
                formatter,
                "box header of {needed_bytes} bytes cut short by an input of {available_bytes}"
            ),
            Self::SizeBelowHeader {
                header_bytes,
                declared_bytes,
            } => write!(
                formatter,
                "box declares a total of {declared_bytes} bytes, below its {header_bytes}-byte \
                 header"
            ),
            Self::TruncatedBox {
                needed_bytes,
                available_bytes,
            } => write!(
                formatter,
                "box of {needed_bytes} bytes cut short by an input of {available_bytes}"
            ),
            Self::BoxTypeMismatch {
                expected_box_type,
                found_box_type,
            } => write!(
                formatter,
                "input holds a {found_box_type} box where a {expected_box_type} box was expected"
            ),
            Self::TruncatedPayload {
                needed_bytes,
                available_bytes,
            } => write!(
                formatter,
                "box payload of {needed_bytes} bytes cut short by an input of {available_bytes}"
            ),
            Self::InvalidValue { field } => write!(
                formatter,
                "box holds a value in its {field} field that the specification does not allow"
            ),
            Self::InvalidUtf8String => {
                formatter.write_str("utf8string field holds bytes that are not UTF-8")
            }
            Self::EntryCountMismatch {
                declared_entries,
                actual_entries,
            } => write!(
                formatter,
                "box declares {declared_entries} entries but holds {actual_entries}"
            ),
            Self::UnsupportedVersion { version } => write!(
                formatter,
                "full box declares version {version}, which this box does not read"
            ),
            Self::UnsupportedFlags { flags } => write!(
                formatter,
                "full box declares flags {flags:#08x}, which this box does not read"
            ),
            Self::UnsupportedValue { field } => write!(
                formatter,
                "box holds a value in its {field} field that this box does not read"
            ),
            Self::TruncatedBuffer {
                needed_bytes,
                available_bytes,
            } => write!(
                formatter,
                "value of {needed_bytes} bytes needs a buffer at least that long, not \
                 {available_bytes}"
            ),
            Self::TrailingBuffer {
                needed_bytes,
                available_bytes,
            } => write!(
                formatter,
                "buffer holds {} bytes past the fields the box wrote",
                available_bytes.saturating_sub(needed_bytes)
            ),
            Self::BufferLengthMismatch {
                declared_bytes,
                offered_bytes,
            } => write!(
                formatter,
                "box payload of {declared_bytes} bytes needs a buffer of that length, not \
                 {offered_bytes}"
            ),
            Self::OutOfRange { value, field_bytes } => write!(
                formatter,
                "value {value} does not fit the {field_bytes} bytes of the field it was given to"
            ),
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
    /// on with the ones it does read, as ISO/IEC 14496-12 §4.2.2 has a reader
    /// skip a full box of a version it does not recognize.
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
    use alloc::string::ToString as _;
    use alloc::vec::Vec;

    use super::Error;
    use crate::codec::field::FieldWidth;
    use crate::data_types::fourcc::FourCc;
    use crate::framing::box_type::BoxType;

    #[test]
    fn a_failure_names_the_boxes_it_was_reached_through_outermost_first() {
        let error = Error::entry_count_mismatch(4, 2)
            .in_container(BoxType::compact(*b"stts"))
            .in_container(BoxType::compact(*b"stbl"));

        assert_eq!(
            error.to_string(),
            "in stbl/stts: box declares 4 entries but holds 2"
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
            .fold(Error::entry_count_mismatch(4, 2), |error, container| {
                error.in_container(BoxType::compact(*container))
            });

        assert_eq!(
            error.to_string(),
            "in .../trak/mdia/minf/stbl/stsd/avc1/btrt/free: box declares 4 entries but holds 2"
        );
    }

    #[test]
    fn an_extended_type_appears_on_the_path_as_its_reserved_code() {
        let error = Error::unsupported_version(1).in_container(BoxType::extended([0xab; 16]));

        assert_eq!(
            error.containers().collect::<Vec<_>>(),
            [FourCc::new(*b"uuid")]
        );
    }

    #[test]
    fn display_of_a_failure_that_names_a_box_names_its_type() {
        assert_eq!(
            Error::box_type_mismatch(BoxType::compact(*b"moov"), BoxType::compact(*b"moof"))
                .to_string(),
            "input holds a moof box where a moov box was expected"
        );
    }

    #[test]
    fn display_names_the_field_a_value_failure_lies_in() {
        assert_eq!(
            [
                Error::invalid_value("sample_description_index").to_string(),
                Error::unsupported_value("field_size").to_string(),
            ],
            [
                "box holds a value in its sample_description_index field that the \
                 specification does not allow",
                "box holds a value in its field_size field that this box does not read",
            ]
        );
    }

    #[test]
    fn display_of_a_failure_a_full_box_declared_names_what_it_declared() {
        assert_eq!(
            [
                Error::unsupported_version(2).to_string(),
                Error::unsupported_flags(0x0000_1000).to_string(),
            ],
            [
                "full box declares version 2, which this box does not read",
                "full box declares flags 0x001000, which this box does not read",
            ]
        );
    }

    #[test]
    fn display_of_a_failure_that_counts_names_both_counts() {
        assert_eq!(
            [
                Error::truncated_header(16, 12).to_string(),
                Error::size_below_header(24, 20).to_string(),
                Error::truncated_box(32, 24).to_string(),
                Error::truncated_payload(20, 12).to_string(),
                Error::trailing_buffer(12, 16).to_string(),
                Error::buffer_length_mismatch(4, 8).to_string(),
                Error::out_of_range(0x1_0000_0000, FieldWidth::Compact).to_string(),
                Error::entry_count_mismatch(4, 2).to_string(),
                Error::truncated_buffer(16, 12).to_string(),
            ],
            [
                "box header of 16 bytes cut short by an input of 12",
                "box declares a total of 20 bytes, below its 24-byte header",
                "box of 32 bytes cut short by an input of 24",
                "box payload of 20 bytes cut short by an input of 12",
                "buffer holds 4 bytes past the fields the box wrote",
                "box payload of 4 bytes needs a buffer of that length, not 8",
                "value 4294967296 does not fit the 4 bytes of the field it was given to",
                "box declares 4 entries but holds 2",
                "value of 16 bytes needs a buffer at least that long, not 12",
            ]
        );
    }
}
