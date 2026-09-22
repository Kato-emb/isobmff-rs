//! [`Error`], the reason a file does not read or write through a demuxer or a muxer

use core::error;
use core::fmt;
use std::io;

/// Reason a file does not read or write through a demuxer or a muxer
///
/// A demuxer or a muxer holds no rule of its own, so what went wrong is one of
/// two failures carried through whole rather than translated: the source or
/// the sink failed, which [`io_error`](Self::io_error) holds as `std::io`
/// reports it, or the layers the file is read or written through refused it,
/// which [`structure_error`](Self::structure_error) holds as
/// [`isobmff_structure::Error`] reports it. Which of the two it is, and what
/// that one makes of it, is one [`kind`](Self::kind).
///
/// # Examples
///
/// ```
/// use std::io;
///
/// use isobmff_core::BoxType;
/// use isobmff_io::{Error, ErrorKind};
///
/// // A failure of the source is carried through as `std::io` reports it
/// let failure = Error::from(io::Error::from(io::ErrorKind::UnexpectedEof));
/// assert_eq!(failure.kind(), ErrorKind::Io(io::ErrorKind::UnexpectedEof));
/// assert_eq!(failure.structure_error(), None);
///
/// // A failure of the layers beneath is carried through as they report it
/// let failure = Error::from(isobmff_structure::Error::missing_mandatory_box(
///     BoxType::compact(*b"moov"),
/// ));
/// assert_eq!(
///     failure.kind(),
///     ErrorKind::Structure(isobmff_structure::ErrorKind::MissingMandatoryBox)
/// );
/// assert!(failure.io_error().is_none());
/// ```
pub struct Error {
    representation: Representation,
}

impl Error {
    /// Returns what went wrong
    #[must_use]
    pub fn kind(&self) -> ErrorKind {
        match &self.representation {
            Representation::Io(failure) => ErrorKind::Io(failure.kind()),
            Representation::Structure(failure) => ErrorKind::Structure(failure.kind()),
        }
    }

    /// Returns the failure of the source or the sink, when it holds one
    #[must_use]
    pub const fn io_error(&self) -> Option<&io::Error> {
        match &self.representation {
            Representation::Io(failure) => Some(failure),
            Representation::Structure(_) => None,
        }
    }

    /// Returns the failure of the layers the file is read or written through, when it holds one
    #[must_use]
    pub const fn structure_error(&self) -> Option<isobmff_structure::Error> {
        match self.representation {
            Representation::Io(_) => None,
            Representation::Structure(failure) => Some(failure),
        }
    }
}

impl From<io::Error> for Error {
    /// Carries the failure of the source or the sink through as it stands
    fn from(failure: io::Error) -> Self {
        Self {
            representation: Representation::Io(failure),
        }
    }
}

impl From<isobmff_structure::Error> for Error {
    /// Carries the failure of the layers beneath through as it stands
    fn from(failure: isobmff_structure::Error) -> Self {
        Self {
            representation: Representation::Structure(failure),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.representation {
            Representation::Io(failure) => write!(formatter, "{failure}"),
            Representation::Structure(failure) => write!(formatter, "{failure}"),
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut fields = formatter.debug_struct("Error");
        fields.field("kind", &self.kind());
        match &self.representation {
            Representation::Io(failure) => fields.field("io_error", failure),
            Representation::Structure(failure) => fields.field("structure_error", failure),
        };

        fields.finish()
    }
}

impl error::Error for Error {
    /// Returns the failure carried through
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.representation {
            Representation::Io(failure) => Some(failure),
            Representation::Structure(failure) => Some(failure),
        }
    }
}

/// What a failure of reading or writing a file through a demuxer or a muxer is
///
/// A demuxer or a muxer names no failure of its own: each kind carries the
/// kind of the failure beneath, so a caller reads what went wrong in one
/// step. Where the failures may come from is added to as the drivers are, so a
/// match on this must leave room for kinds that are not here yet.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ErrorKind {
    /// Failure of the source or the sink, carried through as `std::io` names it
    ///
    /// The failure itself is on [`io_error`](Error::io_error).
    Io(io::ErrorKind),
    /// Failure of the layers the file is read or written through, carried through as `isobmff-structure` names it
    ///
    /// The failure itself is on
    /// [`structure_error`](Error::structure_error).
    Structure(isobmff_structure::ErrorKind),
}

/// The failure carried, keyed by where it came from
enum Representation {
    /// The source or the sink failed
    Io(io::Error),
    /// The layers beneath refused the file
    Structure(isobmff_structure::Error),
}
