//! [`Error`], the reason the sequence of boxes does not read or write

use core::error;
use core::fmt;

use isobmff_core::{BoxType, Category};

/// Reason the sequence of boxes does not read off the input or write into the output
///
/// What went wrong is one [`kind`](Self::kind): a failure of the sequence
/// itself — a file cut short, a call made out of order — or a failure of one
/// box, which [`isobmff_core::Error`] names and this type carries through
/// whole, as [`box_error`](Self::box_error). What a caller does about either
/// is one [`category`](Self::category).
///
/// The values a failure of the sequence carries follow from its kind, and each
/// kind names its own on [`ErrorKind`]. A carried box failure keeps its values
/// and its container path on [`box_error`](Self::box_error), so the accessors
/// here report `None` for it.
///
/// # Examples
///
/// ```
/// use isobmff_core::Category;
/// use isobmff_sequence::{Error, ErrorKind};
///
/// // A failure of the sequence itself names its own kind
/// let failure = Error::unfinished_box(16, 12);
/// assert_eq!(failure.kind(), ErrorKind::UnfinishedBox);
/// assert_eq!(failure.category(), Category::Malformed);
/// assert_eq!(failure.box_error(), None);
///
/// // A failure of one box is carried through whole
/// let carried = Error::from(isobmff_core::Error::unsupported_version(2));
/// assert_eq!(
///     carried.kind(),
///     ErrorKind::Box(isobmff_core::ErrorKind::UnsupportedVersion)
/// );
/// assert_eq!(carried.category(), Category::Unsupported);
/// assert_eq!(
///     carried.box_error().and_then(|box_error| box_error.version()),
///     Some(2)
/// );
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Error {
    representation: Representation,
}

impl Error {
    /// Returns the failure of a file that ends inside the header of a box
    #[must_use]
    pub const fn unfinished_header(needed: u64, available: u64) -> Self {
        Self {
            representation: Representation::UnfinishedHeader { needed, available },
        }
    }

    /// Returns the failure of a file that ends before the total a box declares
    #[must_use]
    pub const fn unfinished_box(needed: u64, available: u64) -> Self {
        Self {
            representation: Representation::UnfinishedBox { needed, available },
        }
    }

    /// Returns the failure of more payload than the total a box declares leaves room for
    #[must_use]
    pub const fn payload_past_declared(box_type: BoxType, declared: u64, offered: u64) -> Self {
        Self {
            representation: Representation::PayloadPastDeclared {
                box_type,
                declared,
                offered,
            },
        }
    }

    /// Returns the failure of a payload, or an end, offered while no box is open
    #[must_use]
    pub const fn no_box_open() -> Self {
        Self {
            representation: Representation::NoBoxOpen,
        }
    }

    /// Returns the failure of a box started while the box before it is still open
    #[must_use]
    pub const fn box_still_open(box_type: BoxType) -> Self {
        Self {
            representation: Representation::BoxStillOpen { box_type },
        }
    }

    /// Returns the failure of something offered after the file was closed off
    #[must_use]
    pub const fn past_end_of_file() -> Self {
        Self {
            representation: Representation::PastEndOfFile,
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
    pub const fn kind(self) -> ErrorKind {
        match self.representation {
            Representation::Box(box_error) => ErrorKind::Box(box_error.kind()),
            Representation::UnfinishedHeader { .. } => ErrorKind::UnfinishedHeader,
            Representation::UnfinishedBox { .. } => ErrorKind::UnfinishedBox,
            Representation::PayloadPastDeclared { .. } => ErrorKind::PayloadPastDeclared,
            Representation::NoBoxOpen => ErrorKind::NoBoxOpen,
            Representation::BoxStillOpen { .. } => ErrorKind::BoxStillOpen,
            Representation::PastEndOfFile => ErrorKind::PastEndOfFile,
            Representation::AlreadyFinished => ErrorKind::AlreadyFinished,
        }
    }

    /// Returns what a caller does about the failure
    #[must_use]
    pub const fn category(self) -> Category {
        match self.representation {
            Representation::Box(box_error) => box_error.category(),
            Representation::UnfinishedHeader { .. } | Representation::UnfinishedBox { .. } => {
                Category::Malformed
            }
            Representation::PayloadPastDeclared { .. }
            | Representation::NoBoxOpen
            | Representation::BoxStillOpen { .. }
            | Representation::PastEndOfFile
            | Representation::AlreadyFinished => Category::Usage,
        }
    }

    /// Returns the failure of one box the sequence carried through, when it holds one
    ///
    /// The values that failure carries, and the boxes it was reached through,
    /// are read off the [`isobmff_core::Error`] itself.
    #[must_use]
    pub const fn box_error(self) -> Option<isobmff_core::Error> {
        match self.representation {
            Representation::Box(box_error) => Some(box_error),
            Representation::UnfinishedHeader { .. }
            | Representation::UnfinishedBox { .. }
            | Representation::PayloadPastDeclared { .. }
            | Representation::NoBoxOpen
            | Representation::BoxStillOpen { .. }
            | Representation::PastEndOfFile
            | Representation::AlreadyFinished => None,
        }
    }

    /// Returns the type of the box the failure names, for the kinds that name one
    #[must_use]
    pub const fn box_type(self) -> Option<BoxType> {
        match self.representation {
            Representation::PayloadPastDeclared { box_type, .. }
            | Representation::BoxStillOpen { box_type } => Some(box_type),
            Representation::Box(_)
            | Representation::UnfinishedHeader { .. }
            | Representation::UnfinishedBox { .. }
            | Representation::NoBoxOpen
            | Representation::PastEndOfFile
            | Representation::AlreadyFinished => None,
        }
    }

    /// Returns the bytes the failure required, for the kinds that count bytes
    #[must_use]
    pub const fn needed_bytes(self) -> Option<u64> {
        match self.representation {
            Representation::UnfinishedHeader { needed, .. }
            | Representation::UnfinishedBox { needed, .. } => Some(needed),
            Representation::PayloadPastDeclared { declared, .. } => Some(declared),
            Representation::Box(_)
            | Representation::NoBoxOpen
            | Representation::BoxStillOpen { .. }
            | Representation::PastEndOfFile
            | Representation::AlreadyFinished => None,
        }
    }

    /// Returns the bytes the failure had to hand, for the kinds that count bytes
    #[must_use]
    pub const fn available_bytes(self) -> Option<u64> {
        match self.representation {
            Representation::UnfinishedHeader { available, .. }
            | Representation::UnfinishedBox { available, .. } => Some(available),
            Representation::PayloadPastDeclared { offered, .. } => Some(offered),
            Representation::Box(_)
            | Representation::NoBoxOpen
            | Representation::BoxStillOpen { .. }
            | Representation::PastEndOfFile
            | Representation::AlreadyFinished => None,
        }
    }
}

impl From<isobmff_core::Error> for Error {
    /// Carries the failure of one box through as it stands
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
            Representation::UnfinishedHeader { needed, available } => write!(
                formatter,
                "file ends {available} bytes into a box header of {needed}"
            ),
            Representation::UnfinishedBox { needed, available } => {
                write!(formatter, "box of {needed} bytes closed off at {available}")
            }
            Representation::PayloadPastDeclared {
                box_type,
                declared,
                offered,
            } => write!(
                formatter,
                "{box_type} box declares {declared} payload bytes, and {offered} were offered"
            ),
            Representation::NoBoxOpen => {
                formatter.write_str("no box is open to carry a payload or an end")
            }
            Representation::BoxStillOpen { box_type } => {
                write!(formatter, "{box_type} box is still open")
            }
            Representation::PastEndOfFile => {
                formatter.write_str("box running to the end of the file was closed already")
            }
            Representation::AlreadyFinished => {
                formatter.write_str("file was declared over and takes nothing more")
            }
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut fields = formatter.debug_struct("Error");
        fields.field("kind", &self.kind());
        fields.field("category", &self.category());

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

impl error::Error for Error {
    /// Returns the failure of one box the sequence carried through, when it holds one
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.representation {
            Representation::Box(box_error) => Some(box_error),
            Representation::UnfinishedHeader { .. }
            | Representation::UnfinishedBox { .. }
            | Representation::PayloadPastDeclared { .. }
            | Representation::NoBoxOpen
            | Representation::BoxStillOpen { .. }
            | Representation::PastEndOfFile
            | Representation::AlreadyFinished => None,
        }
    }
}

/// What a failure of the sequence of boxes is
///
/// The vocabulary is this crate's own: taking a file as it arrives and laying
/// one down name their failures here. A failure of one box — framing it,
/// reading it into a value, or writing one out — is not translated: it keeps
/// the kind [`isobmff_core::ErrorKind`] gives it, carried on
/// [`Box`](Self::Box).
///
/// The situations a sequence reaches are added to as ISO/IEC 14496-12 is read
/// further, so a match on this must leave room for kinds that are not here yet.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ErrorKind {
    /// Failure of one box, carried through as `isobmff-core` names it
    ///
    /// The values that failure carries, and the boxes it was reached through,
    /// are on [`box_error`](Error::box_error).
    Box(isobmff_core::ErrorKind),
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
    /// More payload is offered for a box than the total it declares leaves room for
    ///
    /// [`box_type`](Error::box_type) is the box that declared it,
    /// [`needed_bytes`](Error::needed_bytes) the payload it declares, and
    /// [`available_bytes`](Error::available_bytes) the payload offered
    /// for it.
    PayloadPastDeclared,
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

/// Values a failure carries, keyed by what went wrong
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Representation {
    /// Failure of one box, carried through whole
    Box(isobmff_core::Error),
    /// File that ends inside the header of a box
    UnfinishedHeader { needed: u64, available: u64 },
    /// File that ends before the total a box declares
    UnfinishedBox { needed: u64, available: u64 },
    /// More payload than the total a box declares leaves room for
    PayloadPastDeclared {
        box_type: BoxType,
        declared: u64,
        offered: u64,
    },
    /// Payload, or an end, offered while no box is open
    NoBoxOpen,
    /// Box started while the box before it is still open
    BoxStillOpen { box_type: BoxType },
    /// Something offered after the file was closed off
    PastEndOfFile,
    /// Call made after the file was declared over
    AlreadyFinished,
}

#[cfg(test)]
mod tests {
    use alloc::format;
    use alloc::string::ToString as _;

    use isobmff_core::{BoxType, Category};

    use super::Error;

    #[test]
    fn a_kind_falls_in_the_category_its_situation_asks_for() {
        assert_eq!(
            Error::unfinished_header(16, 9).category(),
            Category::Malformed
        );
        assert_eq!(Error::no_box_open().category(), Category::Usage);
        assert_eq!(
            Error::from(isobmff_core::Error::truncated_header(8, 4)).category(),
            Category::Malformed
        );
    }

    #[test]
    fn a_failure_carries_only_the_values_its_kind_names() {
        let error = Error::payload_past_declared(BoxType::compact(*b"mdat"), 4, 9);

        assert_eq!(error.box_type(), Some(BoxType::compact(*b"mdat")));
        assert_eq!(error.needed_bytes(), Some(4));
        assert_eq!(error.available_bytes(), Some(9));
        assert_eq!(error.box_error(), None);
        assert_eq!(Error::past_end_of_file().box_type(), None);
        assert_eq!(Error::past_end_of_file().needed_bytes(), None);
    }

    #[test]
    fn a_failure_of_one_box_keeps_its_values_and_the_boxes_it_was_reached_through() {
        let box_error =
            isobmff_core::Error::unsupported_version(2).in_container(BoxType::compact(*b"moov"));
        let carried = Error::from(box_error);

        assert_eq!(carried.box_error(), Some(box_error));
        assert_eq!(carried.box_type(), None);
        assert_eq!(carried.needed_bytes(), None);
    }

    #[test]
    fn display_of_a_failure_of_the_sequence_states_the_reason() {
        assert_eq!(
            Error::unfinished_header(16, 9).to_string(),
            "file ends 9 bytes into a box header of 16"
        );
        assert_eq!(
            Error::unfinished_box(16, 12).to_string(),
            "box of 16 bytes closed off at 12"
        );
        assert_eq!(
            Error::payload_past_declared(BoxType::compact(*b"mdat"), 4, 9).to_string(),
            "mdat box declares 4 payload bytes, and 9 were offered"
        );
        assert_eq!(
            Error::no_box_open().to_string(),
            "no box is open to carry a payload or an end"
        );
        assert_eq!(
            Error::box_still_open(BoxType::compact(*b"mdat")).to_string(),
            "mdat box is still open"
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
    fn display_of_a_failure_of_one_box_reads_as_that_failure() {
        let box_error =
            isobmff_core::Error::unsupported_version(2).in_container(BoxType::compact(*b"moov"));

        assert_eq!(Error::from(box_error).to_string(), box_error.to_string());
    }

    #[test]
    fn debug_leaves_out_the_values_a_kind_does_not_carry() {
        let error = Error::already_finished();

        assert_eq!(
            format!("{error:?}"),
            "Error { kind: AlreadyFinished, category: Usage }"
        );
    }

    #[test]
    fn debug_names_the_values_a_kind_carries() {
        let error = Error::payload_past_declared(BoxType::compact(*b"mdat"), 4, 9);

        assert_eq!(
            format!("{error:?}"),
            "Error { kind: PayloadPastDeclared, category: Usage, box_type: Compact(CompactType(FourCC(\"mdat\"))), needed_bytes: 4, available_bytes: 9 }"
        );
    }
}
