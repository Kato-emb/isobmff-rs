//! [`Error`], the reason the sequence of boxes does not read or write

use core::error;
use core::fmt;

use isobmff_core::{BoxType, Category};

/// Reason the sequence of boxes does not read off the input or write into the output
///
/// What went wrong is one variant, which carries the values that describe it:
/// a failure of the sequence itself — a file cut short, a call made out of
/// order — or a failure of one box, which [`isobmff_core::Error`] names and
/// this type carries through whole, as [`Box`](Self::Box). What a caller does
/// about either is one [`category`](Self::category).
///
/// The vocabulary is this crate's own: taking a file as it arrives and laying
/// one down name their failures here. The situations a sequence reaches are
/// added to as ISO/IEC 14496-12 is read further, so a match on this must leave
/// room for kinds that are not here yet, and a match on a kind must leave room
/// for values that are not here yet.
///
/// # Examples
///
/// ```
/// use isobmff_core::Category;
/// use isobmff_sequence::{BoxReader, Error};
///
/// // A file ending inside a box is a failure of the sequence itself
/// let mut reader = BoxReader::new();
/// reader.handle_input(b"\0\0\0\x10freeAAAA").unwrap();
/// let failure = reader.finish().unwrap_err();
/// assert!(matches!(
///     failure,
///     Error::UnfinishedBox {
///         needed_bytes: 16,
///         available_bytes: 12,
///         ..
///     }
/// ));
/// assert_eq!(failure.category(), Category::Malformed);
///
/// // A failure of one box is carried through whole
/// let unsupported = isobmff_core::Error::unsupported_version(2);
/// let carried = Error::from(unsupported);
/// assert!(matches!(carried, Error::Box { 0: box_error, .. } if box_error == unsupported));
/// assert_eq!(carried.category(), Category::Unsupported);
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Error {
    /// Failure of one box, carried through as `isobmff-core` names it
    ///
    /// The values that failure carries, and the boxes it was reached through,
    /// are read off the [`isobmff_core::Error`] itself.
    #[non_exhaustive]
    Box(isobmff_core::Error),
    /// File ends inside the header of a box, with no more input to come
    #[non_exhaustive]
    UnfinishedHeader {
        /// Length the header reaches
        needed_bytes: u64,
        /// Length the file carried
        available_bytes: u64,
    },
    /// Box is closed off before the total it declares is reached
    ///
    /// The file ended inside the box, or the events laying it down closed it
    /// early.
    #[non_exhaustive]
    UnfinishedBox {
        /// Length the box occupies, header included
        needed_bytes: u64,
        /// Length the box was closed off at, header included
        available_bytes: u64,
    },
    /// More payload is offered for a box than the total it declares leaves room for
    #[non_exhaustive]
    PayloadPastDeclared {
        /// Type of the box that declared the total
        box_type: BoxType,
        /// Payload the box declares
        declared_bytes: u64,
        /// Payload offered for the box
        offered_bytes: u64,
    },
    /// Payload, or the end of a box, came while no box was open
    #[non_exhaustive]
    NoBoxOpen,
    /// Box started while the box before it was still open
    #[non_exhaustive]
    BoxStillOpen {
        /// Type of the box left open
        box_type: BoxType,
    },
    /// Something came after the box running to the end of the file was closed
    #[non_exhaustive]
    PastEndOfFile,
    /// File was declared over, and takes nothing more
    #[non_exhaustive]
    AlreadyFinished,
}

impl Error {
    /// Returns what a caller does about the failure
    #[must_use]
    pub const fn category(self) -> Category {
        match self {
            Self::Box(box_error) => box_error.category(),
            Self::UnfinishedHeader { .. } | Self::UnfinishedBox { .. } => Category::Malformed,
            Self::PayloadPastDeclared { .. }
            | Self::NoBoxOpen
            | Self::BoxStillOpen { .. }
            | Self::PastEndOfFile
            | Self::AlreadyFinished => Category::Usage,
        }
    }
}

impl From<isobmff_core::Error> for Error {
    /// Carries the failure of one box through as it stands
    fn from(box_error: isobmff_core::Error) -> Self {
        Self::Box(box_error)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Box(box_error) => write!(formatter, "{box_error}"),
            Self::UnfinishedHeader {
                needed_bytes,
                available_bytes,
            } => write!(
                formatter,
                "file ends {available_bytes} bytes into a box header of {needed_bytes}"
            ),
            Self::UnfinishedBox {
                needed_bytes,
                available_bytes,
            } => write!(
                formatter,
                "box of {needed_bytes} bytes closed off at {available_bytes}"
            ),
            Self::PayloadPastDeclared {
                box_type,
                declared_bytes,
                offered_bytes,
            } => write!(
                formatter,
                "{box_type} box declares {declared_bytes} payload bytes, and {offered_bytes} were \
                 offered"
            ),
            Self::NoBoxOpen => formatter.write_str("no box is open to carry a payload or an end"),
            Self::BoxStillOpen { box_type } => write!(formatter, "{box_type} box is still open"),
            Self::PastEndOfFile => {
                formatter.write_str("box running to the end of the file was closed already")
            }
            Self::AlreadyFinished => {
                formatter.write_str("file was declared over and takes nothing more")
            }
        }
    }
}

impl error::Error for Error {
    /// Returns the failure of one box the sequence carried through, when it holds one
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Box(box_error) => Some(box_error),
            Self::UnfinishedHeader { .. }
            | Self::UnfinishedBox { .. }
            | Self::PayloadPastDeclared { .. }
            | Self::NoBoxOpen
            | Self::BoxStillOpen { .. }
            | Self::PastEndOfFile
            | Self::AlreadyFinished => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString as _;

    use isobmff_core::{BoxType, Category};

    use super::Error;

    #[test]
    fn a_kind_falls_in_the_category_its_situation_asks_for() {
        assert_eq!(
            Error::UnfinishedHeader {
                needed_bytes: 16,
                available_bytes: 9,
            }
            .category(),
            Category::Malformed
        );
        assert_eq!(Error::NoBoxOpen.category(), Category::Usage);
        assert_eq!(
            Error::from(isobmff_core::Error::truncated_header(8, 4)).category(),
            Category::Malformed
        );
    }

    #[test]
    fn display_of_a_failure_of_the_sequence_states_the_reason() {
        assert_eq!(
            Error::UnfinishedHeader {
                needed_bytes: 16,
                available_bytes: 9,
            }
            .to_string(),
            "file ends 9 bytes into a box header of 16"
        );
        assert_eq!(
            Error::UnfinishedBox {
                needed_bytes: 16,
                available_bytes: 12,
            }
            .to_string(),
            "box of 16 bytes closed off at 12"
        );
        assert_eq!(
            Error::PayloadPastDeclared {
                box_type: BoxType::compact(*b"mdat"),
                declared_bytes: 4,
                offered_bytes: 9,
            }
            .to_string(),
            "mdat box declares 4 payload bytes, and 9 were offered"
        );
        assert_eq!(
            Error::NoBoxOpen.to_string(),
            "no box is open to carry a payload or an end"
        );
        assert_eq!(
            Error::BoxStillOpen {
                box_type: BoxType::compact(*b"mdat"),
            }
            .to_string(),
            "mdat box is still open"
        );
        assert_eq!(
            Error::PastEndOfFile.to_string(),
            "box running to the end of the file was closed already"
        );
        assert_eq!(
            Error::AlreadyFinished.to_string(),
            "file was declared over and takes nothing more"
        );
    }

    #[test]
    fn display_of_a_failure_of_one_box_reads_as_that_failure() {
        let box_error =
            isobmff_core::Error::unsupported_version(2).in_container(BoxType::compact(*b"moov"));

        assert_eq!(Error::from(box_error).to_string(), box_error.to_string());
    }
}
