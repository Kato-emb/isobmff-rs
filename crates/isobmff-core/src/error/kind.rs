//! [`ErrorKind`], what a failure of reading or writing a box is, and [`Category`]

use core::fmt;

use crate::framing::box_type::BoxType;

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
