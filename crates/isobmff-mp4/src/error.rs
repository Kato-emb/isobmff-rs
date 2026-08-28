//! [`Error`], the reason a descriptor or the entry holding it does not read or write

use core::error;
use core::fmt;

use isobmff_core::Category;

use crate::descriptor::DescriptorTag;

/// Failure of reading or writing an MPEG-4 sample entry or a descriptor in it
///
/// A failure of a box — a child that does not frame, fields cut short, a
/// buffer too small — arrives as an [`isobmff_core::Error`] and is carried
/// through as [`ErrorKind::Box`]; [`box_error`](Self::box_error) hands it back.
/// The failures the descriptor tree adds are this crate's own, and each kind
/// names its own on [`ErrorKind`].
///
/// # Examples
///
/// ```
/// use isobmff_mp4::{DescriptorTag, ESDBox, Error, ErrorKind};
///
/// // An `esds` whose payload opens with a descriptor other than an ES_Descriptor
/// let failure = ESDBox::decode_payload(b"\0\0\0\0\x04\x00").unwrap_err();
/// assert_eq!(
///     failure.kind(),
///     ErrorKind::DescriptorTagMismatch
/// );
/// assert_eq!(failure.tag(), Some(DescriptorTag::DECODER_CONFIG));
///
/// // A box failure keeps its own kind under `Box`
/// let failure = ESDBox::decode_payload(b"\0\0").unwrap_err();
/// assert_eq!(
///     failure.kind(),
///     ErrorKind::Box(isobmff_core::ErrorKind::TruncatedPayload)
/// );
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Error {
    representation: Representation,
}

impl Error {
    /// Returns the failure of a descriptor found where one of another tag was expected
    #[must_use]
    pub const fn descriptor_tag_mismatch(expected: DescriptorTag, found: DescriptorTag) -> Self {
        Self {
            representation: Representation::DescriptorTagMismatch { expected, found },
        }
    }

    /// Returns the failure of a descriptor that must be present and is not
    #[must_use]
    pub const fn missing_descriptor(tag: DescriptorTag) -> Self {
        Self {
            representation: Representation::MissingDescriptor { tag },
        }
    }

    /// Returns the failure of a descriptor present more often than allowed
    #[must_use]
    pub const fn duplicate_descriptor(tag: DescriptorTag) -> Self {
        Self {
            representation: Representation::DuplicateDescriptor { tag },
        }
    }

    /// Returns the failure of a size that runs past the four bytes an
    /// expandable size may take
    #[must_use]
    pub const fn expandable_size_too_long(tag: DescriptorTag) -> Self {
        Self {
            representation: Representation::ExpandableSizeTooLong { tag },
        }
    }

    /// Returns the kind of the failure
    #[must_use]
    pub const fn kind(self) -> ErrorKind {
        match self.representation {
            Representation::Box(box_error) => ErrorKind::Box(box_error.kind()),
            Representation::DescriptorTagMismatch { .. } => ErrorKind::DescriptorTagMismatch,
            Representation::MissingDescriptor { .. } => ErrorKind::MissingDescriptor,
            Representation::DuplicateDescriptor { .. } => ErrorKind::DuplicateDescriptor,
            Representation::ExpandableSizeTooLong { .. } => ErrorKind::ExpandableSizeTooLong,
        }
    }

    /// Returns the category the failure falls in
    #[must_use]
    pub const fn category(self) -> Category {
        match self.representation {
            Representation::Box(box_error) => box_error.category(),
            Representation::DescriptorTagMismatch { .. }
            | Representation::MissingDescriptor { .. }
            | Representation::DuplicateDescriptor { .. }
            | Representation::ExpandableSizeTooLong { .. } => Category::Malformed,
        }
    }

    /// Returns the box failure carried, for a failure of kind [`ErrorKind::Box`]
    #[must_use]
    pub const fn box_error(self) -> Option<isobmff_core::Error> {
        match self.representation {
            Representation::Box(box_error) => Some(box_error),
            Representation::DescriptorTagMismatch { .. }
            | Representation::MissingDescriptor { .. }
            | Representation::DuplicateDescriptor { .. }
            | Representation::ExpandableSizeTooLong { .. } => None,
        }
    }

    /// Returns the tag of the descriptor the failure is about: the one found,
    /// for a mismatch
    #[must_use]
    pub const fn tag(self) -> Option<DescriptorTag> {
        match self.representation {
            Representation::DescriptorTagMismatch { found, .. } => Some(found),
            Representation::MissingDescriptor { tag }
            | Representation::DuplicateDescriptor { tag }
            | Representation::ExpandableSizeTooLong { tag } => Some(tag),
            Representation::Box(_) => None,
        }
    }

    /// Returns the tag that was expected, for a mismatch
    #[must_use]
    pub const fn expected_tag(self) -> Option<DescriptorTag> {
        match self.representation {
            Representation::DescriptorTagMismatch { expected, .. } => Some(expected),
            Representation::MissingDescriptor { .. }
            | Representation::DuplicateDescriptor { .. }
            | Representation::ExpandableSizeTooLong { .. }
            | Representation::Box(_) => None,
        }
    }
}

impl From<isobmff_core::Error> for Error {
    fn from(box_error: isobmff_core::Error) -> Self {
        Self {
            representation: Representation::Box(box_error),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.representation {
            Representation::Box(box_error) => write!(formatter, "{box_error}"),
            Representation::DescriptorTagMismatch { expected, found } => write!(
                formatter,
                "descriptor with tag {found} found where tag {expected} was expected"
            ),
            Representation::MissingDescriptor { tag } => {
                write!(formatter, "descriptor with tag {tag} is missing")
            }
            Representation::DuplicateDescriptor { tag } => {
                write!(formatter, "descriptor with tag {tag} occurs more than once")
            }
            Representation::ExpandableSizeTooLong { tag } => write!(
                formatter,
                "descriptor with tag {tag} states its size in more than four bytes"
            ),
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Error({:?}: {self})", self.kind())
    }
}

impl error::Error for Error {}

/// Kind of an [`Error`], the situation without the values
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ErrorKind {
    /// A box failed, as [`isobmff_core::Error`] reports it
    Box(isobmff_core::ErrorKind),
    /// A descriptor was found where one of another tag was expected
    DescriptorTagMismatch,
    /// A descriptor that must be present is not
    MissingDescriptor,
    /// A descriptor is present more often than allowed
    DuplicateDescriptor,
    /// An expandable size runs past the four bytes it may take
    ExpandableSizeTooLong,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Representation {
    Box(isobmff_core::Error),
    DescriptorTagMismatch {
        expected: DescriptorTag,
        found: DescriptorTag,
    },
    MissingDescriptor {
        tag: DescriptorTag,
    },
    DuplicateDescriptor {
        tag: DescriptorTag,
    },
    ExpandableSizeTooLong {
        tag: DescriptorTag,
    },
}
