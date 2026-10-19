//! [`DemuxError`], the reason a file does not read through a demuxer

use core::error;
use core::fmt;

use isobmff_core::{BoxType, Category};

/// Reason a file does not read through a demuxer
///
/// What went wrong is one variant, which carries the values that describe it:
/// a failure of the structure of the file — a box it requires that never
/// came, one that came twice, one that came out of the order the structure
/// keeps, a box reaching past the limit the demuxer gathers for one — a
/// call the demuxer does not take — one after the file was declared over,
/// or input at an offset it wants none at — or a failure of a layer beneath,
/// which this type carries through whole rather than translating:
/// [`Sequence`](Self::Sequence) for the framing of the file,
/// [`Sample`](Self::Sample) for the samples it carries, and
/// [`Box`](Self::Box) for one box that did not read. What a caller does about
/// any of them is one [`category`](Self::category).
///
/// The situations a demuxer reaches are added to as ISO/IEC 14496-12 is
/// read further, so a match on this must leave room for variants that are not
/// here yet, and a match on a variant must leave room for fields that are not
/// here yet.
///
/// # Examples
///
/// ```
/// use isobmff::{BoxType, Category};
/// use isobmff::{DemuxError, MovieDemuxer};
///
/// // A file declared over before its movie came is a failure of the structure
/// let mut demuxer = MovieDemuxer::new();
/// let failure = demuxer.finish().unwrap_err();
/// assert!(matches!(
///     failure,
///     DemuxError::MissingMandatoryBox { box_type, .. } if box_type == BoxType::compact(*b"moov")
/// ));
/// assert_eq!(failure.category(), Category::Malformed);
///
/// // A failure of the samples is carried through whole
/// let missing = isobmff::Error::missing_mandatory_box(BoxType::compact(*b"trex"));
/// let sample_error = isobmff::sample::Error::from(missing);
/// let carried = DemuxError::from(sample_error);
/// assert!(matches!(carried, DemuxError::Sample { error, .. } if error == sample_error));
/// assert_eq!(carried.category(), Category::Malformed);
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DemuxError {
    /// Failure of the framing of the file, carried through as [`sequence`](crate::sequence) names it
    #[non_exhaustive]
    Sequence {
        /// Failure of the framing, carrying its own values
        error: crate::sequence::Error,
    },
    /// Failure of the samples the file carries, carried through as [`sample`](crate::sample) names it
    #[non_exhaustive]
    Sample {
        /// Failure of the samples, carrying its own values
        error: crate::sample::Error,
    },
    /// Failure of one box to read, carried through as `isobmff-core` names it
    #[non_exhaustive]
    Box {
        /// Failure of the box, carrying its own values and the boxes it was reached through
        error: isobmff_core::Error,
    },
    /// Box the structure requires is not there
    ///
    /// The file was declared over without it.
    #[non_exhaustive]
    MissingMandatoryBox {
        /// Type of the box that is missing
        box_type: BoxType,
    },
    /// Box the structure carries once is there twice
    #[non_exhaustive]
    DuplicateBox {
        /// Type of the box that came again
        box_type: BoxType,
    },
    /// Box lies out of the order the structure keeps
    ///
    /// It came before a box the structure places ahead of it, or after one it
    /// places behind it.
    #[non_exhaustive]
    BoxOutOfOrder {
        /// Type of the box that came out of order
        box_type: BoxType,
    },
    /// Box read whole reaches past the limit the demuxer gathers
    #[non_exhaustive]
    PayloadLimitExceeded {
        /// Type of the box
        box_type: BoxType,
        /// Length of payload the box declares, or for a box declaring no total, the length it has reached
        reached_bytes: u64,
        /// Length of payload the demuxer gathers for one box at most
        limit_bytes: u64,
    },
    /// File was declared over, and takes nothing more until the reading restarts
    #[non_exhaustive]
    AlreadyFinished,
    /// Input was handed over at an offset the demuxer takes no input at
    ///
    /// The demuxer takes input where it names the read it wants and, while
    /// it takes the input in order, where that input stands, and refuses it
    /// anywhere else.
    #[non_exhaustive]
    UnwantedInput {
        /// Offset the input was handed over at
        input_offset: u64,
    },
}

impl DemuxError {
    /// Returns what a caller does about the failure
    #[must_use]
    pub const fn category(self) -> Category {
        match self {
            Self::Sequence { error } => error.category(),
            Self::Sample { error } => error.category(),
            Self::Box { error } => error.category(),
            Self::MissingMandatoryBox { .. }
            | Self::DuplicateBox { .. }
            | Self::BoxOutOfOrder { .. } => Category::Malformed,
            Self::PayloadLimitExceeded { .. } => Category::Unsupported,
            Self::AlreadyFinished | Self::UnwantedInput { .. } => Category::Usage,
        }
    }
}

impl From<crate::sequence::Error> for DemuxError {
    /// Carries the failure of the framing of the file through as it stands
    fn from(error: crate::sequence::Error) -> Self {
        Self::Sequence { error }
    }
}

impl From<crate::sample::Error> for DemuxError {
    /// Carries the failure of the samples through as it stands
    fn from(error: crate::sample::Error) -> Self {
        Self::Sample { error }
    }
}

impl From<isobmff_core::Error> for DemuxError {
    /// Carries the failure of one box through as it stands
    fn from(error: isobmff_core::Error) -> Self {
        Self::Box { error }
    }
}

impl From<crate::structure::Error> for DemuxError {
    /// Carries the failure of the structure through as the variant of the same name
    fn from(error: crate::structure::Error) -> Self {
        match error {
            crate::structure::Error::MissingMandatoryBox { box_type } => {
                Self::MissingMandatoryBox { box_type }
            }
            crate::structure::Error::DuplicateBox { box_type } => Self::DuplicateBox { box_type },
            crate::structure::Error::BoxOutOfOrder { box_type } => Self::BoxOutOfOrder { box_type },
        }
    }
}

impl fmt::Display for DemuxError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Sequence { error } => write!(formatter, "{error}"),
            Self::Sample { error } => write!(formatter, "{error}"),
            Self::Box { error } => write!(formatter, "{error}"),
            Self::MissingMandatoryBox { box_type } => {
                write!(formatter, "file carries no {box_type} box")
            }
            Self::DuplicateBox { box_type } => {
                write!(formatter, "file carries a second {box_type} box")
            }
            Self::BoxOutOfOrder { box_type } => write!(
                formatter,
                "file carries a {box_type} box out of the order its structure keeps"
            ),
            Self::PayloadLimitExceeded {
                box_type,
                reached_bytes,
                limit_bytes,
            } => write!(
                formatter,
                "{box_type} box reaches {reached_bytes} payload bytes, past the {limit_bytes}-byte \
                 limit"
            ),
            Self::AlreadyFinished => {
                formatter.write_str("file was declared over and takes nothing more")
            }
            Self::UnwantedInput { input_offset } => write!(
                formatter,
                "input handed over at offset {input_offset} is not the read the demuxer wants"
            ),
        }
    }
}

impl error::Error for DemuxError {
    /// Returns the failure of the layer beneath, when it holds one
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Sequence { error } => Some(error),
            Self::Sample { error } => Some(error),
            Self::Box { error } => Some(error),
            Self::MissingMandatoryBox { .. }
            | Self::DuplicateBox { .. }
            | Self::BoxOutOfOrder { .. }
            | Self::PayloadLimitExceeded { .. }
            | Self::AlreadyFinished
            | Self::UnwantedInput { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString as _;

    use isobmff_core::{BoxType, Category};

    use super::DemuxError;

    /// Box the failures in these tests name
    const MOOV: BoxType = BoxType::compact(*b"moov");

    #[test]
    fn a_failure_falls_in_the_category_its_situation_asks_for() {
        assert_eq!(
            DemuxError::MissingMandatoryBox { box_type: MOOV }.category(),
            Category::Malformed
        );
        assert_eq!(
            DemuxError::PayloadLimitExceeded {
                box_type: MOOV,
                reached_bytes: 32,
                limit_bytes: 16,
            }
            .category(),
            Category::Unsupported
        );
        assert_eq!(DemuxError::AlreadyFinished.category(), Category::Usage);
        assert_eq!(
            DemuxError::UnwantedInput { input_offset: 9 }.category(),
            Category::Usage
        );
        assert_eq!(
            DemuxError::from(isobmff_core::Error::unsupported_version(2)).category(),
            Category::Unsupported
        );
    }

    #[test]
    fn display_of_a_failure_of_the_structure_states_the_reason() {
        assert_eq!(
            DemuxError::MissingMandatoryBox { box_type: MOOV }.to_string(),
            "file carries no moov box"
        );
        assert_eq!(
            DemuxError::DuplicateBox { box_type: MOOV }.to_string(),
            "file carries a second moov box"
        );
        assert_eq!(
            DemuxError::BoxOutOfOrder { box_type: MOOV }.to_string(),
            "file carries a moov box out of the order its structure keeps"
        );
        assert_eq!(
            DemuxError::PayloadLimitExceeded {
                box_type: MOOV,
                reached_bytes: 32,
                limit_bytes: 16,
            }
            .to_string(),
            "moov box reaches 32 payload bytes, past the 16-byte limit"
        );
        assert_eq!(
            DemuxError::AlreadyFinished.to_string(),
            "file was declared over and takes nothing more"
        );
        assert_eq!(
            DemuxError::UnwantedInput { input_offset: 9 }.to_string(),
            "input handed over at offset 9 is not the read the demuxer wants"
        );
    }

    #[test]
    fn display_of_a_carried_failure_reads_as_that_failure() {
        let sample_error = crate::sample::Error::from(isobmff_core::Error::unsupported_version(2));

        assert_eq!(
            DemuxError::from(sample_error).to_string(),
            sample_error.to_string()
        );
    }
}
