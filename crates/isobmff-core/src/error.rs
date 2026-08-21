//! [`Error`], the reason a box does not read off bytes or write into them

use core::error;
use core::fmt;

use crate::codec::field::FieldWidth;
use crate::data_types::fourcc::FourCC;
use crate::framing::box_type::BoxType;

/// Boxes a failure holds of the path out of the containers it was read in
const CONTAINER_DEPTH: usize = 8;

/// Returns a length of bytes as the count a failure carries
pub(crate) fn byte_count(length: usize) -> u64 {
    // Why not unwrap: a usize above `u64::MAX` needs a 128-bit target to exist,
    // and saturating keeps the panic-free path.
    u64::try_from(length).unwrap_or(u64::MAX)
}

/// Reason a box does not read off bytes, or does not write into them
///
/// What went wrong is one [`kind`](Self::kind), and what a caller does about it
/// is one [`category`](Self::category). Which of the values a failure carries
/// are there follows from its kind, and each kind names its own on
/// [`ErrorKind`].
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
/// // What went wrong, and what a caller does about it
/// assert_eq!(failure.kind(), ErrorKind::UnsupportedVersion);
/// assert_eq!(failure.category(), Category::Unsupported);
/// assert_eq!(failure.version(), Some(2));
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
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Error {
    kind: ErrorKind,
    containers: [Option<FourCC>; CONTAINER_DEPTH],
    dropped_containers: bool,
    box_type: Option<BoxType>,
    detail: Detail,
}

impl Error {
    /// Returns a failure of `kind` carrying `detail`
    const fn new(kind: ErrorKind, detail: Detail) -> Self {
        Self {
            kind,
            containers: [None; CONTAINER_DEPTH],
            dropped_containers: false,
            box_type: None,
            detail,
        }
    }

    /// Returns a failure of `kind` about the box `box_type` names, counting bytes
    const fn about_bytes(kind: ErrorKind, box_type: BoxType, needed: u64, available: u64) -> Self {
        Self {
            kind,
            containers: [None; CONTAINER_DEPTH],
            dropped_containers: false,
            box_type: Some(box_type),
            detail: Detail::Bytes { needed, available },
        }
    }

    /// Returns a failure of `kind` about the box `box_type` names
    const fn about(kind: ErrorKind, box_type: BoxType) -> Self {
        Self {
            kind,
            containers: [None; CONTAINER_DEPTH],
            dropped_containers: false,
            box_type: Some(box_type),
            detail: Detail::Nothing,
        }
    }

    /// Returns the failure of an input that ends inside the header of a box
    #[must_use]
    pub const fn truncated_header(needed: u64, available: u64) -> Self {
        Self::new(
            ErrorKind::TruncatedHeader,
            Detail::Bytes { needed, available },
        )
    }

    /// Returns the failure of a box declaring a total below the header it prefixes
    #[must_use]
    pub const fn size_below_header(header_len: u64, declared: u64) -> Self {
        Self::new(
            ErrorKind::SizeBelowHeader,
            Detail::Bytes {
                needed: header_len,
                available: declared,
            },
        )
    }

    /// Returns the failure of a box whose declared total overruns the input
    #[must_use]
    pub const fn truncated_box(needed: u64, available: u64) -> Self {
        Self::new(ErrorKind::TruncatedBox, Detail::Bytes { needed, available })
    }

    /// Returns the failure of a file that ends inside the header of a box
    #[must_use]
    pub const fn unfinished_header(needed: u64, available: u64) -> Self {
        Self::new(
            ErrorKind::UnfinishedHeader,
            Detail::Bytes { needed, available },
        )
    }

    /// Returns the failure of a file that ends before the total a box declares
    #[must_use]
    pub const fn unfinished_box(needed: u64, available: u64) -> Self {
        Self::new(
            ErrorKind::UnfinishedBox,
            Detail::Bytes { needed, available },
        )
    }

    /// Returns the failure of a box read as a type the input does not hold there
    #[must_use]
    pub const fn box_type_mismatch(expected: BoxType, found: BoxType) -> Self {
        Self {
            detail: Detail::FoundBoxType(found),
            ..Self::about(ErrorKind::BoxTypeMismatch, expected)
        }
    }

    /// Returns the failure of a payload that ends inside a field
    #[must_use]
    pub const fn truncated_payload(needed: u64, available: u64) -> Self {
        Self::new(
            ErrorKind::TruncatedPayload,
            Detail::Bytes { needed, available },
        )
    }

    /// Returns the failure of a payload holding bytes past the fields it reads
    #[must_use]
    pub const fn trailing_payload(needed: u64, available: u64) -> Self {
        Self::new(
            ErrorKind::TrailingPayload,
            Detail::Bytes { needed, available },
        )
    }

    /// Returns the failure of more payload than the total a box declares leaves room for
    #[must_use]
    pub const fn payload_past_declared(box_type: BoxType, declared: u64, offered: u64) -> Self {
        Self::about_bytes(ErrorKind::PayloadPastDeclared, box_type, declared, offered)
    }

    /// Returns the failure of a box declaring a payload past the limit a reader holds
    #[must_use]
    pub const fn payload_limit_exceeded(box_type: BoxType, declared: u64, limit: u64) -> Self {
        Self::about_bytes(ErrorKind::PayloadLimitExceeded, box_type, declared, limit)
    }

    /// Returns the failure of a buffer that ends inside what is written into it
    #[must_use]
    pub const fn truncated_buffer(needed: u64, available: u64) -> Self {
        Self::new(
            ErrorKind::TruncatedBuffer,
            Detail::Bytes { needed, available },
        )
    }

    /// Returns the failure of a buffer holding bytes past the fields a box wrote
    #[must_use]
    pub const fn trailing_buffer(needed: u64, available: u64) -> Self {
        Self::new(
            ErrorKind::TrailingBuffer,
            Detail::Bytes { needed, available },
        )
    }

    /// Returns the failure of a buffer that is not the length a payload declared
    #[must_use]
    pub const fn buffer_length_mismatch(declared: u64, offered: u64) -> Self {
        Self::new(
            ErrorKind::BufferLengthMismatch,
            Detail::Bytes {
                needed: declared,
                available: offered,
            },
        )
    }

    /// Returns the failure of a value wider than the field it was given to
    #[must_use]
    pub const fn out_of_range(value: u64, width: FieldWidth) -> Self {
        let field_bytes = match width {
            FieldWidth::Compact => 4,
            FieldWidth::Extended => 8,
        };

        Self::new(
            ErrorKind::OutOfRange,
            Detail::OutOfRange {
                value,
                width: field_bytes,
            },
        )
    }

    /// Returns the failure of a full box declaring flags the spec forbids together
    #[must_use]
    pub const fn conflicting_flags(flags: u32) -> Self {
        Self::new(ErrorKind::ConflictingFlags, Detail::Flags(flags))
    }

    /// Returns the failure of a text field that does not read as UTF-8
    #[must_use]
    pub const fn invalid_utf8(valid_up_to: usize) -> Self {
        Self::new(ErrorKind::InvalidUtf8, Detail::ValidUpTo(valid_up_to))
    }

    /// Returns the failure of a container lacking a child the spec marks mandatory
    #[must_use]
    pub const fn missing_mandatory_box(box_type: BoxType) -> Self {
        Self::about(ErrorKind::MissingMandatoryBox, box_type)
    }

    /// Returns the failure of a container holding more of a child than it may
    #[must_use]
    pub const fn duplicate_box(box_type: BoxType) -> Self {
        Self::about(ErrorKind::DuplicateBox, box_type)
    }

    /// Returns the failure of a container holding a child a field of it forbids
    #[must_use]
    pub const fn forbidden_child_box(box_type: BoxType) -> Self {
        Self::about(ErrorKind::ForbiddenChildBox, box_type)
    }

    /// Returns the failure of a count that disagrees with the entries it frames
    #[must_use]
    pub const fn entry_count_mismatch(declared: u64, actual: u64) -> Self {
        Self::new(
            ErrorKind::EntryCountMismatch,
            Detail::Entries {
                needed: declared,
                available: actual,
            },
        )
    }

    /// Returns the failure of a full box declaring a version the box does not read
    #[must_use]
    pub const fn unsupported_version(version: u8) -> Self {
        Self::new(ErrorKind::UnsupportedVersion, Detail::Version(version))
    }

    /// Returns the failure of a full box declaring flags the box does not read
    #[must_use]
    pub const fn unsupported_flags(flags: u32) -> Self {
        Self::new(ErrorKind::UnsupportedFlags, Detail::Flags(flags))
    }

    /// Returns the failure of a count past the entries a box reads
    #[must_use]
    pub const fn unsupported_entry_count(declared: u64, limit: u64) -> Self {
        Self::new(
            ErrorKind::UnsupportedEntryCount,
            Detail::Entries {
                needed: declared,
                available: limit,
            },
        )
    }

    /// Returns the failure of a payload, or an end, offered while no box is open
    #[must_use]
    pub const fn no_box_open() -> Self {
        Self::new(ErrorKind::NoBoxOpen, Detail::Nothing)
    }

    /// Returns the failure of a box started while the box before it is still open
    #[must_use]
    pub const fn box_still_open(box_type: BoxType) -> Self {
        Self::about(ErrorKind::BoxStillOpen, box_type)
    }

    /// Returns the failure of something offered after the file was closed off
    #[must_use]
    pub const fn past_end_of_file() -> Self {
        Self::new(ErrorKind::PastEndOfFile, Detail::Nothing)
    }

    /// Returns the failure of a call made after the file was declared over
    #[must_use]
    pub const fn already_finished() -> Self {
        Self::new(ErrorKind::AlreadyFinished, Detail::Nothing)
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

    /// Returns what went wrong
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

    /// Returns the type of the box the failure names, for the kinds that name one
    #[must_use]
    pub const fn box_type(self) -> Option<BoxType> {
        self.box_type
    }

    /// Returns the type the input holds, for the kinds that name what was there
    #[must_use]
    pub const fn found_box_type(self) -> Option<BoxType> {
        match self.detail {
            Detail::FoundBoxType(found) => Some(found),
            Detail::Nothing
            | Detail::Bytes { .. }
            | Detail::Entries { .. }
            | Detail::Version(_)
            | Detail::Flags(_)
            | Detail::OutOfRange { .. }
            | Detail::ValidUpTo(_) => None,
        }
    }

    /// Returns the bytes the failure required, for the kinds that count bytes
    ///
    /// For [`OutOfRange`](ErrorKind::OutOfRange) this is the width of the field
    /// the value did not fit.
    #[must_use]
    pub const fn needed_bytes(self) -> Option<u64> {
        match self.detail {
            Detail::Bytes { needed, .. } => Some(needed),
            Detail::OutOfRange { width, .. } => Some(width),
            Detail::Nothing
            | Detail::Entries { .. }
            | Detail::Version(_)
            | Detail::Flags(_)
            | Detail::ValidUpTo(_)
            | Detail::FoundBoxType(_) => None,
        }
    }

    /// Returns the bytes the failure had to hand, for the kinds that count bytes
    ///
    /// For [`SizeBelowHeader`](ErrorKind::SizeBelowHeader) this is the total the
    /// `size` or `largesize` field declared.
    #[must_use]
    pub const fn available_bytes(self) -> Option<u64> {
        match self.detail {
            Detail::Bytes { available, .. } => Some(available),
            Detail::Nothing
            | Detail::Entries { .. }
            | Detail::Version(_)
            | Detail::Flags(_)
            | Detail::OutOfRange { .. }
            | Detail::ValidUpTo(_)
            | Detail::FoundBoxType(_) => None,
        }
    }

    /// Returns the entries the failure required, for the kinds that count entries
    #[must_use]
    pub const fn needed_entries(self) -> Option<u64> {
        match self.detail {
            Detail::Entries { needed, .. } => Some(needed),
            Detail::Nothing
            | Detail::Bytes { .. }
            | Detail::Version(_)
            | Detail::Flags(_)
            | Detail::OutOfRange { .. }
            | Detail::ValidUpTo(_)
            | Detail::FoundBoxType(_) => None,
        }
    }

    /// Returns the entries the failure had to hand, for the kinds that count entries
    #[must_use]
    pub const fn available_entries(self) -> Option<u64> {
        match self.detail {
            Detail::Entries { available, .. } => Some(available),
            Detail::Nothing
            | Detail::Bytes { .. }
            | Detail::Version(_)
            | Detail::Flags(_)
            | Detail::OutOfRange { .. }
            | Detail::ValidUpTo(_)
            | Detail::FoundBoxType(_) => None,
        }
    }

    /// Returns the version a full box declared, for the kinds that name one
    #[must_use]
    pub const fn version(self) -> Option<u8> {
        match self.detail {
            Detail::Version(version) => Some(version),
            Detail::Nothing
            | Detail::Bytes { .. }
            | Detail::Entries { .. }
            | Detail::Flags(_)
            | Detail::OutOfRange { .. }
            | Detail::ValidUpTo(_)
            | Detail::FoundBoxType(_) => None,
        }
    }

    /// Returns the flags a full box declared, for the kinds that name them
    #[must_use]
    pub const fn flags(self) -> Option<u32> {
        match self.detail {
            Detail::Flags(flags) => Some(flags),
            Detail::Nothing
            | Detail::Bytes { .. }
            | Detail::Entries { .. }
            | Detail::Version(_)
            | Detail::OutOfRange { .. }
            | Detail::ValidUpTo(_)
            | Detail::FoundBoxType(_) => None,
        }
    }

    /// Returns the value a field was given, for the kinds that name one
    #[must_use]
    pub const fn value(self) -> Option<u64> {
        match self.detail {
            Detail::OutOfRange { value, .. } => Some(value),
            Detail::Nothing
            | Detail::Bytes { .. }
            | Detail::Entries { .. }
            | Detail::Version(_)
            | Detail::Flags(_)
            | Detail::ValidUpTo(_)
            | Detail::FoundBoxType(_) => None,
        }
    }

    /// Returns the text that read before the byte that did not, in bytes
    #[must_use]
    pub const fn valid_up_to(self) -> Option<usize> {
        match self.detail {
            Detail::ValidUpTo(valid_up_to) => Some(valid_up_to),
            Detail::Nothing
            | Detail::Bytes { .. }
            | Detail::Entries { .. }
            | Detail::Version(_)
            | Detail::Flags(_)
            | Detail::OutOfRange { .. }
            | Detail::FoundBoxType(_) => None,
        }
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

        let needed = self.needed_bytes().unwrap_or_default();
        let available = self.available_bytes().unwrap_or_default();
        let named = Named(self.box_type);
        let found = Named(self.found_box_type());
        match self.kind {
            ErrorKind::TruncatedHeader => write!(
                formatter,
                "box header of {needed} bytes cut short by an input of {available}"
            ),
            ErrorKind::SizeBelowHeader => write!(
                formatter,
                "box declares a total of {available} bytes, below its {needed}-byte header"
            ),
            ErrorKind::TruncatedBox => write!(
                formatter,
                "box of {needed} bytes cut short by an input of {available}"
            ),
            ErrorKind::UnfinishedHeader => write!(
                formatter,
                "file ends {available} bytes into a box header of {needed}"
            ),
            ErrorKind::UnfinishedBox => {
                write!(formatter, "box of {needed} bytes closed off at {available}")
            }
            ErrorKind::BoxTypeMismatch => write!(
                formatter,
                "input holds a {found}box where a {named}box was expected"
            ),
            ErrorKind::TruncatedPayload => write!(
                formatter,
                "box payload of {needed} bytes cut short by an input of {available}"
            ),
            ErrorKind::TrailingPayload => write!(
                formatter,
                "box payload leaves {} bytes past the fields it holds",
                available.saturating_sub(needed)
            ),
            ErrorKind::PayloadPastDeclared => write!(
                formatter,
                "{named}box declares {needed} payload bytes, and {available} were offered"
            ),
            ErrorKind::PayloadLimitExceeded => write!(
                formatter,
                "{named}box declares {needed} payload bytes, past the {available}-byte limit"
            ),
            ErrorKind::NoBoxOpen => {
                formatter.write_str("no box is open to carry a payload or an end")
            }
            ErrorKind::BoxStillOpen => write!(formatter, "{named}box is still open"),
            ErrorKind::PastEndOfFile => {
                formatter.write_str("box running to the end of the file was closed already")
            }
            ErrorKind::AlreadyFinished => {
                formatter.write_str("file was declared over and takes nothing more")
            }
            ErrorKind::TruncatedBuffer => write!(
                formatter,
                "value of {needed} bytes needs a buffer at least that long, not {available}"
            ),
            ErrorKind::TrailingBuffer => write!(
                formatter,
                "buffer holds {} bytes past the fields the box wrote",
                available.saturating_sub(needed)
            ),
            ErrorKind::BufferLengthMismatch => write!(
                formatter,
                "box payload of {needed} bytes needs a buffer of that length, not {available}"
            ),
            ErrorKind::OutOfRange => write!(
                formatter,
                "value {} does not fit the {needed} bytes of the field it was given to",
                self.value().unwrap_or_default()
            ),
            ErrorKind::ConflictingFlags => write!(
                formatter,
                "full box declares flags {:#08x}, which the spec does not allow together",
                self.flags().unwrap_or_default()
            ),
            ErrorKind::InvalidUtf8 => {
                formatter.write_str("box payload holds a string that is not UTF-8")
            }
            ErrorKind::MissingMandatoryBox => {
                write!(formatter, "container holds no mandatory {named}box")
            }
            ErrorKind::DuplicateBox => write!(
                formatter,
                "container holds more than one {named}box, which may appear once"
            ),
            ErrorKind::ForbiddenChildBox => write!(
                formatter,
                "container holds a {named}box that a field of it forbids"
            ),
            ErrorKind::EntryCountMismatch => write!(
                formatter,
                "box declares {} entries but holds {}",
                self.needed_entries().unwrap_or_default(),
                self.available_entries().unwrap_or_default()
            ),
            ErrorKind::UnsupportedVersion => write!(
                formatter,
                "full box declares version {}, which this box does not read",
                self.version().unwrap_or_default()
            ),
            ErrorKind::UnsupportedFlags => write!(
                formatter,
                "full box declares flags {:#08x}, which this box does not read",
                self.flags().unwrap_or_default()
            ),
            ErrorKind::UnsupportedEntryCount => write!(
                formatter,
                "box declares {} entries, past the {} this box reads",
                self.needed_entries().unwrap_or_default(),
                self.available_entries().unwrap_or_default()
            ),
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut fields = formatter.debug_struct("Error");
        fields.field("kind", &self.kind);
        fields.field("category", &self.category());

        if self.dropped_containers || self.containers().next().is_some() {
            fields.field("containers", &Containers(*self));
        }
        if let Some(box_type) = self.box_type {
            fields.field("box_type", &box_type);
        }

        match self.detail {
            Detail::Nothing => {}
            Detail::Bytes { needed, available } => {
                fields.field("needed_bytes", &needed);
                fields.field("available_bytes", &available);
            }
            Detail::Entries { needed, available } => {
                fields.field("needed_entries", &needed);
                fields.field("available_entries", &available);
            }
            Detail::Version(version) => {
                fields.field("version", &version);
            }
            Detail::Flags(flags) => {
                fields.field("flags", &flags);
            }
            Detail::OutOfRange { value, width } => {
                fields.field("value", &value);
                fields.field("needed_bytes", &width);
            }
            Detail::ValidUpTo(valid_up_to) => {
                fields.field("valid_up_to", &valid_up_to);
            }
            Detail::FoundBoxType(found) => {
                fields.field("found_box_type", &found);
            }
        }

        fields.finish()
    }
}

impl error::Error for Error {}

/// What a failure of reading or writing a box is
///
/// The vocabulary is the whole of it: reading one box off a slice, laying a
/// sequence of them down, and the calls a caller makes in the wrong order all
/// name their failure here. Each kind states which of the values an
/// [`Error`] carries it brings, and falls in one [`Category`],
/// which [`Error::category`](Error::category) reports.
///
/// The situations a box reaches are added to as ISO/IEC 14496-12 is read
/// further, so a match on this must leave room for kinds that are not here yet.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ErrorKind {
    /// Input ends inside the header of a box
    ///
    /// [`needed_bytes`](Error::needed_bytes) is the length the header
    /// reaches, [`available_bytes`](Error::available_bytes) the length
    /// the input offered.
    TruncatedHeader,
    /// Total a box declares is smaller than the header it prefixes
    ///
    /// [`needed_bytes`](Error::needed_bytes) is the length the header
    /// occupies, [`available_bytes`](Error::available_bytes) the total
    /// the `size` or `largesize` field declares.
    SizeBelowHeader,
    /// Total a box declares overruns the input
    ///
    /// [`needed_bytes`](Error::needed_bytes) is the length the box
    /// occupies, [`available_bytes`](Error::available_bytes) the length
    /// the input offered.
    TruncatedBox,
    /// File ends inside the header of a box, with no more input to come
    ///
    /// [`needed_bytes`](Error::needed_bytes) is the length the header
    /// reaches, [`available_bytes`](Error::available_bytes) the length
    /// the file carried.
    UnfinishedHeader,
    /// Box is closed off before the total it declares is reached
    ///
    /// The file ended inside the box, or the events laying it down closed it
    /// early. [`needed_bytes`](Error::needed_bytes) is the length the box
    /// occupies, [`available_bytes`](Error::available_bytes) the length
    /// it was closed off at, header included.
    UnfinishedBox,
    /// Box read as one type is of another
    ///
    /// [`box_type`](Error::box_type) is the type the box was read as,
    /// [`found_box_type`](Error::found_box_type) the type the input holds.
    BoxTypeMismatch,
    /// Payload of a box ends inside a field
    ///
    /// [`needed_bytes`](Error::needed_bytes) is the length the fields
    /// read so far require, [`available_bytes`](Error::available_bytes)
    /// the length the payload offered.
    TruncatedPayload,
    /// Payload of a box holds bytes past the fields it reads
    ///
    /// [`needed_bytes`](Error::needed_bytes) is the length the fields
    /// took, [`available_bytes`](Error::available_bytes) the length the
    /// payload holds.
    TrailingPayload,
    /// More payload is offered for a box than the total it declares leaves room for
    ///
    /// [`box_type`](Error::box_type) is the box that declared it,
    /// [`needed_bytes`](Error::needed_bytes) the payload it declares, and
    /// [`available_bytes`](Error::available_bytes) the payload offered
    /// for it.
    PayloadPastDeclared,
    /// Full box declares flags the spec does not allow together
    ///
    /// [`flags`](Error::flags) is the flags the box declares.
    ConflictingFlags,
    /// Field the spec declares as text does not read as UTF-8
    ///
    /// [`valid_up_to`](Error::valid_up_to) is the length of text that
    /// reads before the byte that does not.
    InvalidUtf8,
    /// Container lacks a child box the spec marks mandatory
    ///
    /// [`box_type`](Error::box_type) is the type of the child that is
    /// missing.
    MissingMandatoryBox,
    /// Container holds more of a child box than its quantity allows
    ///
    /// [`box_type`](Error::box_type) is the type of the child held more
    /// than once.
    DuplicateBox,
    /// Container holds a child box that a field of it forbids
    ///
    /// [`box_type`](Error::box_type) is the type of the child that is
    /// forbidden.
    ForbiddenChildBox,
    /// Count a box declares does not match the entries it frames for itself
    ///
    /// [`needed_entries`](Error::needed_entries) is the count the
    /// `entry_count` field declares,
    /// [`available_entries`](Error::available_entries) the count the
    /// payload holds.
    EntryCountMismatch,
    /// Full box declares a version the box does not read
    ///
    /// [`version`](Error::version) is the version the box declares.
    UnsupportedVersion,
    /// Full box declares flags the box does not read
    ///
    /// [`flags`](Error::flags) is the flags the box declares.
    UnsupportedFlags,
    /// Count a box declares is past the entries the box reads
    ///
    /// [`needed_entries`](Error::needed_entries) is the count the field
    /// declares, [`available_entries`](Error::available_entries) the
    /// count the box reads.
    UnsupportedEntryCount,
    /// Box read into a value declares a payload past the limit the reader holds
    ///
    /// [`box_type`](Error::box_type) is the box that declared it,
    /// [`needed_bytes`](Error::needed_bytes) the payload it declares, and
    /// [`available_bytes`](Error::available_bytes) the payload the reader
    /// gathers for one box at most.
    PayloadLimitExceeded,
    /// Buffer ends inside the value being written into it
    ///
    /// [`needed_bytes`](Error::needed_bytes) is the length the value
    /// requires, [`available_bytes`](Error::available_bytes) the length
    /// the buffer offered.
    TruncatedBuffer,
    /// Buffer holds bytes past the fields a box wrote
    ///
    /// [`needed_bytes`](Error::needed_bytes) is the length the fields
    /// wrote, [`available_bytes`](Error::available_bytes) the length the
    /// buffer holds.
    TrailingBuffer,
    /// Buffer offered for a payload is not the length the payload declared
    ///
    /// [`needed_bytes`](Error::needed_bytes) is the length the payload
    /// declares, [`available_bytes`](Error::available_bytes) the length
    /// the buffer offered.
    BufferLengthMismatch,
    /// Value is wider than the field it was given to
    ///
    /// [`value`](Error::value) is the value the field was given,
    /// [`needed_bytes`](Error::needed_bytes) the width of that field.
    OutOfRange,
    /// Payload, or the end of a box, came while no box was open
    NoBoxOpen,
    /// Box started while the box before it was still open
    ///
    /// [`box_type`](Error::box_type) is the box left open.
    BoxStillOpen,
    /// Something came after the box running to the end of the file was closed
    PastEndOfFile,
    /// File was declared over, and takes nothing more
    AlreadyFinished,
}

impl ErrorKind {
    /// Returns what a caller does about a failure of this kind
    pub(crate) const fn category(self) -> Category {
        match self {
            Self::TruncatedHeader
            | Self::SizeBelowHeader
            | Self::TruncatedBox
            | Self::UnfinishedHeader
            | Self::UnfinishedBox
            | Self::BoxTypeMismatch
            | Self::TruncatedPayload
            | Self::TrailingPayload
            | Self::ConflictingFlags
            | Self::InvalidUtf8
            | Self::MissingMandatoryBox
            | Self::DuplicateBox
            | Self::ForbiddenChildBox
            | Self::EntryCountMismatch => Category::Malformed,
            Self::UnsupportedVersion
            | Self::UnsupportedFlags
            | Self::UnsupportedEntryCount
            | Self::PayloadLimitExceeded => Category::Unsupported,
            Self::TruncatedBuffer
            | Self::TrailingBuffer
            | Self::BufferLengthMismatch
            | Self::OutOfRange
            | Self::PayloadPastDeclared
            | Self::NoBoxOpen
            | Self::BoxStillOpen
            | Self::PastEndOfFile
            | Self::AlreadyFinished => Category::Usage,
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
    /// The file being read is malformed, or the events offered would lay down
    /// one that is.
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

/// Box type a failure names, as `Display` writes it before the word `box`
///
/// A failure that names no box leaves the word standing on its own, so the line
/// reads either way.
struct Named(Option<BoxType>);

impl fmt::Display for Named {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(box_type) => write!(formatter, "{box_type} "),
            None => Ok(()),
        }
    }
}

/// Boxes a failure was reached through, as `Debug` lists them
struct Containers(Error);

impl fmt::Debug for Containers {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut list = formatter.debug_list();
        if self.0.dropped_containers {
            list.entry(&"...");
        }

        list.entries(self.0.containers()).finish()
    }
}

/// Values a failure carries, as its kind calls for
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Detail {
    /// Kind that stands on its own, or names a box and nothing more
    Nothing,
    /// Bytes required against bytes to hand
    Bytes { needed: u64, available: u64 },
    /// Entries required against entries to hand
    Entries { needed: u64, available: u64 },
    /// Version a full box declared
    Version(u8),
    /// Flags a full box declared
    Flags(u32),
    /// Value a field was given, against the bytes of that field
    OutOfRange { value: u64, width: u64 },
    /// Text that read before the byte that did not, in bytes
    ValidUpTo(usize),
    /// Box type an input holds where another was to be read
    FoundBoxType(BoxType),
}

#[cfg(test)]
mod tests {
    use alloc::format;
    use alloc::string::ToString as _;
    use alloc::vec;
    use alloc::vec::Vec;

    use super::{Category, Error};
    use crate::codec::field::FieldWidth;
    use crate::data_types::fourcc::FourCC;
    use crate::framing::box_type::BoxType;

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
    fn a_failure_carries_only_the_values_its_kind_names() {
        let error = Error::out_of_range(0x1_0000_0000, FieldWidth::Compact);

        assert_eq!(error.value(), Some(0x1_0000_0000));
        assert_eq!(error.needed_bytes(), Some(4));
        assert_eq!(error.available_bytes(), None);
        assert_eq!(error.version(), None);
    }

    #[test]
    fn a_kind_falls_in_the_category_its_situation_asks_for() {
        assert_eq!(Error::truncated_box(32, 24).category(), Category::Malformed);
        assert_eq!(
            Error::unsupported_version(2).category(),
            Category::Unsupported
        );
        assert_eq!(Error::no_box_open().category(), Category::Usage);
        assert_eq!(
            Error::buffer_length_mismatch(4, 8).category(),
            Category::Usage
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
    fn a_failure_that_counts_entries_carries_both_counts() {
        let error = Error::entry_count_mismatch(4, 2);

        assert_eq!(error.needed_entries(), Some(4));
        assert_eq!(error.available_entries(), Some(2));
        assert_eq!(error.needed_bytes(), None);
    }

    #[test]
    fn a_failure_about_one_box_names_its_type() {
        let error = Error::missing_mandatory_box(BoxType::compact(*b"mvhd"));

        assert_eq!(error.box_type(), Some(BoxType::compact(*b"mvhd")));
        assert_eq!(Error::no_box_open().box_type(), None);
    }

    #[test]
    fn a_failure_of_a_box_read_as_another_type_names_both_types() {
        let error =
            Error::box_type_mismatch(BoxType::compact(*b"moov"), BoxType::compact(*b"moof"));

        assert_eq!(error.box_type(), Some(BoxType::compact(*b"moov")));
        assert_eq!(error.found_box_type(), Some(BoxType::compact(*b"moof")));
        assert_eq!(Error::no_box_open().found_box_type(), None);
    }

    #[test]
    fn a_failure_of_a_full_box_carries_what_the_box_declared() {
        assert_eq!(Error::unsupported_version(2).version(), Some(2));
        assert_eq!(
            Error::conflicting_flags(0x0000_0404).flags(),
            Some(0x0000_0404)
        );
        assert_eq!(Error::invalid_utf8(3).valid_up_to(), Some(3));
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
            Error::box_still_open(BoxType::compact(*b"mdat")).to_string(),
            "mdat box is still open"
        );
        assert_eq!(
            Error::payload_past_declared(BoxType::compact(*b"mdat"), 4, 9).to_string(),
            "mdat box declares 4 payload bytes, and 9 were offered"
        );
        assert_eq!(
            Error::payload_limit_exceeded(BoxType::compact(*b"moov"), 32, 16).to_string(),
            "moov box declares 32 payload bytes, past the 16-byte limit"
        );
        assert_eq!(
            Error::box_type_mismatch(BoxType::compact(*b"moov"), BoxType::compact(*b"moof"))
                .to_string(),
            "input holds a moof box where a moov box was expected"
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
            Error::invalid_utf8(3).to_string(),
            "box payload holds a string that is not UTF-8"
        );
    }

    #[test]
    fn display_of_a_failure_that_counts_entries_names_both_counts() {
        assert_eq!(
            Error::entry_count_mismatch(4, 2).to_string(),
            "box declares 4 entries but holds 2"
        );
        assert_eq!(
            Error::unsupported_entry_count(u64::from(u32::MAX), 1_048_576).to_string(),
            "box declares 4294967295 entries, past the 1048576 this box reads"
        );
    }

    #[test]
    fn display_of_a_call_the_api_does_not_take_says_which_call_it_was() {
        assert_eq!(
            Error::no_box_open().to_string(),
            "no box is open to carry a payload or an end"
        );
        assert_eq!(
            Error::past_end_of_file().to_string(),
            "box running to the end of the file was closed already"
        );
        assert_eq!(
            Error::already_finished().to_string(),
            "file was declared over and takes nothing more"
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
            Error::unfinished_header(16, 9).to_string(),
            "file ends 9 bytes into a box header of 16"
        );
        assert_eq!(
            Error::unfinished_box(16, 12).to_string(),
            "box of 16 bytes closed off at 12"
        );
        assert_eq!(
            Error::out_of_range(0x1_0000_0000, FieldWidth::Compact).to_string(),
            "value 4294967296 does not fit the 4 bytes of the field it was given to"
        );
    }

    #[test]
    fn debug_leaves_out_the_values_a_kind_does_not_carry() {
        let error = Error::unsupported_version(2);

        assert_eq!(
            format!("{error:?}"),
            "Error { kind: UnsupportedVersion, category: Unsupported, version: 2 }"
        );
    }
}
