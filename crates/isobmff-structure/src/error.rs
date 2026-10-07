//! [`Error`], the reason a file does not read through the layers this crate holds

use core::error;
use core::fmt;

use isobmff_core::{BoxType, Category};

/// Reason a file does not read through the layers this crate holds
///
/// What went wrong is one variant, which carries the values that describe it:
/// a failure of the structure of the file — a box it requires that never
/// came, one that came twice, one that came out of the order the structure
/// keeps, a box reaching past the limit a demux FSM gathers for one, brands a
/// mux FSM cannot write under — or a failure of a layer beneath, which this
/// type carries through whole rather than translating:
/// [`Sequence`](Self::Sequence) for the framing of the file,
/// [`Sample`](Self::Sample) for the samples it carries, and
/// [`Box`](Self::Box) for one box that did not decode. What a caller does
/// about any of them is one [`category`](Self::category).
///
/// The vocabulary is this crate's own: the boxes a structure is made of, the
/// order it keeps them in, and reading one of them whole name their failures
/// here. The situations a structure reaches are added to as ISO/IEC 14496-12
/// is read further, so a match on this must leave room for variants that are
/// not here yet, and a match on a variant must leave room for fields that are
/// not here yet.
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxType, Category};
/// use isobmff_structure::{Error, MovieDemuxFsm};
///
/// // A file declared over before its movie came is a failure of the structure
/// let mut demux_fsm = MovieDemuxFsm::new();
/// let failure = demux_fsm.finish().unwrap_err();
/// assert!(matches!(
///     failure,
///     Error::MissingMandatoryBox { box_type, .. } if box_type == BoxType::compact(*b"moov")
/// ));
/// assert_eq!(failure.category(), Category::Malformed);
///
/// // A failure of the samples is carried through whole
/// let missing = isobmff_core::Error::missing_mandatory_box(BoxType::compact(*b"trex"));
/// let sample_error = isobmff_sample::Error::from(missing);
/// let carried = Error::from(sample_error);
/// assert!(matches!(carried, Error::Sample { error, .. } if error == sample_error));
/// assert_eq!(carried.category(), Category::Malformed);
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Error {
    /// Failure of the framing of the file, carried through as `isobmff-sequence` names it
    #[non_exhaustive]
    Sequence {
        /// Failure of the framing, carrying its own values
        error: isobmff_sequence::Error,
    },
    /// Failure of the samples the file carries, carried through as `isobmff-sample` names it
    #[non_exhaustive]
    Sample {
        /// Failure of the samples, carrying its own values
        error: isobmff_sample::Error,
    },
    /// Failure of one box, carried through as `isobmff-core` names it
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
    /// Box read whole reaches past the limit the demux FSM gathers
    #[non_exhaustive]
    PayloadLimitExceeded {
        /// Type of the box
        box_type: BoxType,
        /// Length of payload the box declares, or for a box declaring no total, the length it has reached
        reached_bytes: u64,
        /// Length of payload the demux FSM gathers for one box at most
        limit_bytes: u64,
    },
    /// Brands were handed over that the mux FSM cannot write under
    ///
    /// A mux FSM that writes `default-base-is-moof` in every `tfhd` refuses a
    /// `ftyp` or `styp` listing any brand earlier than `iso5` — `isom`,
    /// `avc1`, `iso2`, `iso3` or `iso4` — under which ISO/IEC 14496-12
    /// §8.8.7.1 forbids the flag.
    #[non_exhaustive]
    UnsupportedBrand,
    /// File was declared over, and takes nothing more
    #[non_exhaustive]
    AlreadyFinished,
    /// Input was handed over at an offset the demux FSM takes no input at
    ///
    /// The demux FSM takes input where it names the read it wants and, while
    /// it takes the input in order, where that input stands, and refuses it
    /// anywhere else.
    #[non_exhaustive]
    UnwantedInput {
        /// Offset the input was handed over at
        input_offset: u64,
    },
}

impl Error {
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
            Self::PayloadLimitExceeded { .. } | Self::UnsupportedBrand => Category::Unsupported,
            Self::AlreadyFinished | Self::UnwantedInput { .. } => Category::Usage,
        }
    }
}

impl From<isobmff_sequence::Error> for Error {
    /// Carries the failure of the framing of the file through as it stands
    fn from(error: isobmff_sequence::Error) -> Self {
        Self::Sequence { error }
    }
}

impl From<isobmff_sample::Error> for Error {
    /// Carries the failure of the samples through as it stands
    fn from(error: isobmff_sample::Error) -> Self {
        Self::Sample { error }
    }
}

impl From<isobmff_core::Error> for Error {
    /// Carries the failure of one box through as it stands
    fn from(error: isobmff_core::Error) -> Self {
        Self::Box { error }
    }
}

impl fmt::Display for Error {
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
            Self::UnsupportedBrand => {
                formatter.write_str("brands handed over are ones the mux FSM cannot write under")
            }
            Self::AlreadyFinished => {
                formatter.write_str("file was declared over and takes nothing more")
            }
            Self::UnwantedInput { input_offset } => write!(
                formatter,
                "input handed over at offset {input_offset} is not the read the demux FSM wants"
            ),
        }
    }
}

impl error::Error for Error {
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
            | Self::UnsupportedBrand
            | Self::AlreadyFinished
            | Self::UnwantedInput { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString as _;

    use isobmff_core::{BoxType, Category};

    use super::Error;

    /// The `moov` box, which most of the structure's failures name
    const MOOV: BoxType = BoxType::compact(*b"moov");

    #[test]
    fn a_failure_falls_in_the_category_its_situation_asks_for() {
        assert_eq!(
            Error::MissingMandatoryBox { box_type: MOOV }.category(),
            Category::Malformed
        );
        assert_eq!(
            Error::PayloadLimitExceeded {
                box_type: MOOV,
                reached_bytes: 32,
                limit_bytes: 16,
            }
            .category(),
            Category::Unsupported
        );
        assert_eq!(Error::UnsupportedBrand.category(), Category::Unsupported);
        assert_eq!(Error::AlreadyFinished.category(), Category::Usage);
        assert_eq!(
            Error::UnwantedInput { input_offset: 9 }.category(),
            Category::Usage
        );
        assert_eq!(
            Error::from(isobmff_core::Error::unsupported_version(2)).category(),
            Category::Unsupported
        );
    }

    #[test]
    fn display_of_a_failure_of_the_structure_states_the_reason() {
        assert_eq!(
            Error::MissingMandatoryBox { box_type: MOOV }.to_string(),
            "file carries no moov box"
        );
        assert_eq!(
            Error::DuplicateBox { box_type: MOOV }.to_string(),
            "file carries a second moov box"
        );
        assert_eq!(
            Error::BoxOutOfOrder { box_type: MOOV }.to_string(),
            "file carries a moov box out of the order its structure keeps"
        );
        assert_eq!(
            Error::PayloadLimitExceeded {
                box_type: MOOV,
                reached_bytes: 32,
                limit_bytes: 16,
            }
            .to_string(),
            "moov box reaches 32 payload bytes, past the 16-byte limit"
        );
        assert_eq!(
            Error::UnsupportedBrand.to_string(),
            "brands handed over are ones the mux FSM cannot write under"
        );
        assert_eq!(
            Error::AlreadyFinished.to_string(),
            "file was declared over and takes nothing more"
        );
        assert_eq!(
            Error::UnwantedInput { input_offset: 9 }.to_string(),
            "input handed over at offset 9 is not the read the demux FSM wants"
        );
    }

    #[test]
    fn display_of_a_carried_failure_reads_as_that_failure() {
        let sample_error = isobmff_sample::Error::from(isobmff_core::Error::unsupported_version(2));

        assert_eq!(
            Error::from(sample_error).to_string(),
            sample_error.to_string()
        );
    }
}
