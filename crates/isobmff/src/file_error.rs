//! [`FileError`], the reason a fragmented file does not read or write

use core::error;
use core::fmt;

use isobmff_core::{BoxType, Category};

use crate::SampleError;

/// Reason a fragmented file does not read or write
///
/// What went wrong is one [`kind`](Self::kind): a failure of the layout itself
/// — a box the layout requires that never came, one that came twice, a box
/// declaring more payload than the reader gathers — or a failure of a layer
/// beneath, which this type carries through whole rather than translating:
/// [`sequence_error`](Self::sequence_error) for the framing of the file,
/// [`sample_error`](Self::sample_error) for the samples its fragments carry,
/// and [`box_error`](Self::box_error) for one box that did not decode. What a
/// caller does about any of them is one [`category`](Self::category).
///
/// The values a failure of the layout carries follow from its kind, and each
/// kind names its own on [`FileErrorKind`]. A carried failure keeps its own
/// values, so the accessors here report `None` for it.
///
/// # Examples
///
/// ```
/// use isobmff::{BoxType, Category, FileError, FileErrorKind, SampleError, SampleErrorKind};
///
/// // A failure of the layout names its own kind
/// let failure = FileError::missing_mandatory_box(BoxType::compact(*b"moov"));
/// assert_eq!(failure.kind(), FileErrorKind::MissingMandatoryBox);
/// assert_eq!(failure.category(), Category::Malformed);
/// assert_eq!(failure.box_type(), Some(BoxType::compact(*b"moov")));
///
/// // A failure of the samples is carried through whole
/// let carried = FileError::from(SampleError::unknown_track_id(3));
/// assert_eq!(
///     carried.kind(),
///     FileErrorKind::Sample(SampleErrorKind::UnknownTrackId)
/// );
/// assert_eq!(
///     carried.sample_error().map(SampleError::kind),
///     Some(SampleErrorKind::UnknownTrackId)
/// );
/// assert_eq!(carried.box_type(), None);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileError {
    representation: Representation,
}

impl FileError {
    /// Returns the failure of a file lacking a box its layout requires
    #[must_use]
    pub const fn missing_mandatory_box(box_type: BoxType) -> Self {
        Self {
            representation: Representation::MissingBox {
                box_type,
                origin: Origin::File,
            },
        }
    }

    /// Returns the failure of a file holding a box its layout carries once
    #[must_use]
    pub const fn duplicate_box(box_type: BoxType) -> Self {
        Self {
            representation: Representation::DuplicateBox {
                box_type,
                origin: Origin::File,
            },
        }
    }

    /// Returns the failure of a box the layout requires before what was handed over
    #[must_use]
    pub const fn box_not_handed_over(box_type: BoxType) -> Self {
        Self {
            representation: Representation::MissingBox {
                box_type,
                origin: Origin::Caller,
            },
        }
    }

    /// Returns the failure of a box handed over where the layout carries one
    #[must_use]
    pub const fn box_handed_over_twice(box_type: BoxType) -> Self {
        Self {
            representation: Representation::DuplicateBox {
                box_type,
                origin: Origin::Caller,
            },
        }
    }

    /// Returns the failure of a box declaring a payload past the limit a reader holds
    #[must_use]
    pub const fn payload_limit_exceeded(box_type: BoxType, declared: u64, limit: u64) -> Self {
        Self {
            representation: Representation::PayloadLimitExceeded {
                box_type,
                declared,
                limit,
            },
        }
    }

    /// Returns the failure of a call made after the file was declared over
    #[must_use]
    pub const fn already_finished() -> Self {
        Self {
            representation: Representation::AlreadyFinished,
        }
    }

    /// Returns what went wrong
    #[must_use]
    pub const fn kind(self) -> FileErrorKind {
        match self.representation {
            Representation::Sequence(failure) => FileErrorKind::Sequence(failure.kind()),
            Representation::Sample(failure) => FileErrorKind::Sample(failure.kind()),
            Representation::Box(box_error) => FileErrorKind::Box(box_error.kind()),
            Representation::MissingBox { .. } => FileErrorKind::MissingMandatoryBox,
            Representation::DuplicateBox { .. } => FileErrorKind::DuplicateBox,
            Representation::PayloadLimitExceeded { .. } => FileErrorKind::PayloadLimitExceeded,
            Representation::AlreadyFinished => FileErrorKind::AlreadyFinished,
        }
    }

    /// Returns what a caller does about the failure
    #[must_use]
    pub const fn category(self) -> Category {
        match self.representation {
            Representation::Sequence(failure) => failure.category(),
            Representation::Sample(failure) => failure.category(),
            Representation::Box(box_error) => box_error.category(),
            Representation::MissingBox { origin, .. }
            | Representation::DuplicateBox { origin, .. } => match origin {
                Origin::File => Category::Malformed,
                Origin::Caller => Category::Usage,
            },
            Representation::PayloadLimitExceeded { .. } => Category::Unsupported,
            Representation::AlreadyFinished => Category::Usage,
        }
    }

    /// Returns the failure of the framing of the file, when it holds one
    #[must_use]
    pub const fn sequence_error(self) -> Option<isobmff_sequence::Error> {
        match self.representation {
            Representation::Sequence(failure) => Some(failure),
            Representation::Sample(_)
            | Representation::Box(_)
            | Representation::MissingBox { .. }
            | Representation::DuplicateBox { .. }
            | Representation::PayloadLimitExceeded { .. }
            | Representation::AlreadyFinished => None,
        }
    }

    /// Returns the failure of the samples the fragments carry, when it holds one
    #[must_use]
    pub const fn sample_error(self) -> Option<SampleError> {
        match self.representation {
            Representation::Sample(failure) => Some(failure),
            Representation::Sequence(_)
            | Representation::Box(_)
            | Representation::MissingBox { .. }
            | Representation::DuplicateBox { .. }
            | Representation::PayloadLimitExceeded { .. }
            | Representation::AlreadyFinished => None,
        }
    }

    /// Returns the failure of one box the file carried through, when it holds one
    ///
    /// The values that failure carries, and the boxes it was reached through,
    /// are read off the [`isobmff_core::Error`] itself.
    #[must_use]
    pub const fn box_error(self) -> Option<isobmff_core::Error> {
        match self.representation {
            Representation::Box(box_error) => Some(box_error),
            Representation::Sequence(_)
            | Representation::Sample(_)
            | Representation::MissingBox { .. }
            | Representation::DuplicateBox { .. }
            | Representation::PayloadLimitExceeded { .. }
            | Representation::AlreadyFinished => None,
        }
    }

    /// Returns the type of the box the failure names, for the kinds that name one
    #[must_use]
    pub const fn box_type(self) -> Option<BoxType> {
        match self.representation {
            Representation::MissingBox { box_type, .. }
            | Representation::DuplicateBox { box_type, .. }
            | Representation::PayloadLimitExceeded { box_type, .. } => Some(box_type),
            Representation::Sequence(_)
            | Representation::Sample(_)
            | Representation::Box(_)
            | Representation::AlreadyFinished => None,
        }
    }

    /// Returns the bytes the failure required, for the kinds that count bytes
    #[must_use]
    pub const fn needed_bytes(self) -> Option<u64> {
        match self.representation {
            Representation::PayloadLimitExceeded { declared, .. } => Some(declared),
            Representation::Sequence(_)
            | Representation::Sample(_)
            | Representation::Box(_)
            | Representation::MissingBox { .. }
            | Representation::DuplicateBox { .. }
            | Representation::AlreadyFinished => None,
        }
    }

    /// Returns the bytes the failure had to hand, for the kinds that count bytes
    #[must_use]
    pub const fn available_bytes(self) -> Option<u64> {
        match self.representation {
            Representation::PayloadLimitExceeded { limit, .. } => Some(limit),
            Representation::Sequence(_)
            | Representation::Sample(_)
            | Representation::Box(_)
            | Representation::MissingBox { .. }
            | Representation::DuplicateBox { .. }
            | Representation::AlreadyFinished => None,
        }
    }
}

impl From<isobmff_sequence::Error> for FileError {
    /// Carries the failure of the framing of the file through as it stands
    fn from(failure: isobmff_sequence::Error) -> Self {
        Self {
            representation: Representation::Sequence(failure),
        }
    }
}

impl From<SampleError> for FileError {
    /// Carries the failure of the samples through as it stands
    fn from(failure: SampleError) -> Self {
        Self {
            representation: Representation::Sample(failure),
        }
    }
}

impl From<isobmff_core::Error> for FileError {
    /// Carries the failure of one box through as it stands
    fn from(box_error: isobmff_core::Error) -> Self {
        Self {
            representation: Representation::Box(box_error),
        }
    }
}

impl fmt::Display for FileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.representation {
            Representation::Sequence(failure) => write!(formatter, "{failure}"),
            Representation::Sample(failure) => write!(formatter, "{failure}"),
            Representation::Box(box_error) => write!(formatter, "{box_error}"),
            Representation::MissingBox { box_type, origin } => match origin {
                Origin::File => write!(formatter, "file carries no {box_type} box"),
                Origin::Caller => write!(formatter, "no {box_type} box was handed over first"),
            },
            Representation::DuplicateBox { box_type, origin } => match origin {
                Origin::File => write!(formatter, "file carries a second {box_type} box"),
                Origin::Caller => write!(formatter, "a {box_type} box was handed over already"),
            },
            Representation::PayloadLimitExceeded {
                box_type,
                declared,
                limit,
            } => write!(
                formatter,
                "{box_type} box declares {declared} payload bytes, past the {limit}-byte limit"
            ),
            Representation::AlreadyFinished => {
                formatter.write_str("file was declared over and takes nothing more")
            }
        }
    }
}

impl fmt::Debug for FileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut fields = formatter.debug_struct("FileError");
        fields.field("kind", &self.kind());
        fields.field("category", &self.category());

        if let Some(failure) = self.sequence_error() {
            fields.field("sequence_error", &failure);
        }
        if let Some(failure) = self.sample_error() {
            fields.field("sample_error", &failure);
        }
        if let Some(box_error) = self.box_error() {
            fields.field("box_error", &box_error);
        }
        if let Some(box_type) = self.box_type() {
            fields.field("box_type", &box_type);
        }
        if let Some(needed) = self.needed_bytes() {
            fields.field("needed_bytes", &needed);
        }
        if let Some(available) = self.available_bytes() {
            fields.field("available_bytes", &available);
        }

        fields.finish()
    }
}

impl error::Error for FileError {
    /// Returns the failure of the layer beneath the file, when it holds one
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.representation {
            Representation::Sequence(failure) => Some(failure),
            Representation::Sample(failure) => Some(failure),
            Representation::Box(box_error) => Some(box_error),
            Representation::MissingBox { .. }
            | Representation::DuplicateBox { .. }
            | Representation::PayloadLimitExceeded { .. }
            | Representation::AlreadyFinished => None,
        }
    }
}

/// What a failure of a fragmented file is
///
/// The vocabulary is this layer's own: the boxes a layout is made of, the order
/// it puts them in, and the payload a reader gathers for one of them name their
/// failures here. A failure of a layer beneath is not translated: it keeps the
/// kind that layer gives it, carried on [`Sequence`](Self::Sequence),
/// [`Sample`](Self::Sample), or [`Box`](Self::Box).
///
/// The situations a layout reaches are added to as ISO/IEC 14496-12 is read
/// further, so a match on this must leave room for kinds that are not here yet.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FileErrorKind {
    /// Failure of the framing of the file, carried through as `isobmff-sequence` names it
    ///
    /// The values that failure carries are on
    /// [`sequence_error`](FileError::sequence_error).
    Sequence(isobmff_sequence::ErrorKind),
    /// Failure of the samples the fragments carry, carried through as the sample layer names it
    ///
    /// The values that failure carries are on
    /// [`sample_error`](FileError::sample_error).
    Sample(crate::SampleErrorKind),
    /// Failure of one box, carried through as `isobmff-core` names it
    ///
    /// The values that failure carries, and the boxes it was reached through,
    /// are on [`box_error`](FileError::box_error).
    Box(isobmff_core::ErrorKind),
    /// Box the layout requires is not there
    ///
    /// The file carries none of it, or the caller writing one has not handed it
    /// over yet — [`category`](FileError::category) tells the two apart.
    /// [`box_type`](FileError::box_type) is the box that is missing.
    MissingMandatoryBox,
    /// Box the layout carries once is there twice
    ///
    /// [`box_type`](FileError::box_type) is the box that came again.
    DuplicateBox,
    /// Box the layout reads into a value declares a payload past the limit the reader holds
    ///
    /// [`box_type`](FileError::box_type) is the box that declared it,
    /// [`needed_bytes`](FileError::needed_bytes) the payload it declares, and
    /// [`available_bytes`](FileError::available_bytes) the payload the reader
    /// gathers for one box at most.
    PayloadLimitExceeded,
    /// File was declared over, and takes nothing more
    AlreadyFinished,
}

/// Values a failure carries, keyed by what went wrong
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Representation {
    /// Failure of the framing of the file, carried through whole
    Sequence(isobmff_sequence::Error),
    /// Failure of the samples the fragments carry, carried through whole
    Sample(SampleError),
    /// Failure of one box, carried through whole
    Box(isobmff_core::Error),
    /// Box the layout requires that is not there
    MissingBox { box_type: BoxType, origin: Origin },
    /// Box the layout carries once that is there twice
    DuplicateBox { box_type: BoxType, origin: Origin },
    /// Box declaring a payload past the limit a reader holds
    PayloadLimitExceeded {
        box_type: BoxType,
        declared: u64,
        limit: u64,
    },
    /// Call made after the file was declared over
    AlreadyFinished,
}

/// Which side of the layout a box went missing on, or came again on
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Origin {
    /// The file that was read, which the reader cannot mend
    File,
    /// The calls laying one down, which the caller can still mend
    Caller,
}
