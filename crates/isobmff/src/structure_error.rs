//! [`StructureError`], the reason a file does not read through the layers this crate holds

use core::error;
use core::fmt;

use isobmff_core::{BoxType, Category};

/// Reason a file does not read through the layers this crate holds
///
/// What went wrong is one [`kind`](Self::kind): a failure of the structure of
/// the file — a box it requires that never came, one that came twice, one
/// that came out of the order the structure keeps — a failure of reading a
/// box whole — a payload past the limit the reader gathers, a box handed over
/// while another was open — or a failure of one box, which
/// [`isobmff_core::Error`] names and this type carries through whole, as
/// [`box_error`](Self::box_error). What a caller does about any of them is one
/// [`category`](Self::category).
///
/// The values a failure of this crate's own carries follow from its kind, and
/// each kind names its own on [`StructureErrorKind`]. A carried box failure
/// keeps its values and its container path on [`box_error`](Self::box_error),
/// so the accessors here report `None` for it.
///
/// # Examples
///
/// ```
/// use isobmff::{BoxType, Category, ErrorKind, StructureError, StructureErrorKind};
///
/// // A failure of the structure names its own kind
/// let failure = StructureError::missing_mandatory_box(BoxType::compact(*b"moov"));
/// assert_eq!(failure.kind(), StructureErrorKind::MissingMandatoryBox);
/// assert_eq!(failure.category(), Category::Malformed);
/// assert_eq!(failure.box_type(), Some(BoxType::compact(*b"moov")));
///
/// // A failure of one box is carried through whole
/// let carried = StructureError::from(isobmff::Error::unsupported_version(2));
/// assert_eq!(
///     carried.kind(),
///     StructureErrorKind::Box(ErrorKind::UnsupportedVersion)
/// );
/// assert_eq!(carried.box_error().and_then(|box_error| box_error.version()), Some(2));
/// assert_eq!(carried.box_type(), None);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct StructureError {
    representation: Representation,
}

impl StructureError {
    /// Returns the failure of a file lacking a box its structure requires
    #[must_use]
    pub const fn missing_mandatory_box(box_type: BoxType) -> Self {
        Self {
            representation: Representation::MissingMandatoryBox { box_type },
        }
    }

    /// Returns the failure of a file holding a box its structure carries once
    #[must_use]
    pub const fn duplicate_box(box_type: BoxType) -> Self {
        Self {
            representation: Representation::DuplicateBox { box_type },
        }
    }

    /// Returns the failure of a box lying out of the order the structure keeps
    #[must_use]
    pub const fn box_out_of_order(box_type: BoxType) -> Self {
        Self {
            representation: Representation::BoxOutOfOrder { box_type },
        }
    }

    /// Returns the failure of a box reaching past the limit a reader gathers
    #[must_use]
    pub const fn payload_limit_exceeded(box_type: BoxType, reached: u64, limit: u64) -> Self {
        Self {
            representation: Representation::PayloadLimitExceeded {
                box_type,
                reached,
                limit,
            },
        }
    }

    /// Returns the failure of payload, or the end of a box, handed over while no box was open
    #[must_use]
    pub const fn no_box_open() -> Self {
        Self {
            representation: Representation::NoBoxOpen,
        }
    }

    /// Returns the failure of a box handed over while the box before it was still open
    #[must_use]
    pub const fn box_still_open(box_type: BoxType) -> Self {
        Self {
            representation: Representation::BoxStillOpen { box_type },
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
    pub const fn kind(self) -> StructureErrorKind {
        match self.representation {
            Representation::Box(box_error) => StructureErrorKind::Box(box_error.kind()),
            Representation::MissingMandatoryBox { .. } => StructureErrorKind::MissingMandatoryBox,
            Representation::DuplicateBox { .. } => StructureErrorKind::DuplicateBox,
            Representation::BoxOutOfOrder { .. } => StructureErrorKind::BoxOutOfOrder,
            Representation::PayloadLimitExceeded { .. } => StructureErrorKind::PayloadLimitExceeded,
            Representation::NoBoxOpen => StructureErrorKind::NoBoxOpen,
            Representation::BoxStillOpen { .. } => StructureErrorKind::BoxStillOpen,
            Representation::AlreadyFinished => StructureErrorKind::AlreadyFinished,
        }
    }

    /// Returns what a caller does about the failure
    #[must_use]
    pub const fn category(self) -> Category {
        match self.representation {
            Representation::Box(box_error) => box_error.category(),
            Representation::MissingMandatoryBox { .. }
            | Representation::DuplicateBox { .. }
            | Representation::BoxOutOfOrder { .. } => Category::Malformed,
            Representation::PayloadLimitExceeded { .. } => Category::Unsupported,
            Representation::NoBoxOpen
            | Representation::BoxStillOpen { .. }
            | Representation::AlreadyFinished => Category::Usage,
        }
    }

    /// Returns the failure of one box carried through, when it holds one
    ///
    /// The values that failure carries, and the boxes it was reached through,
    /// are read off the [`isobmff_core::Error`] itself.
    #[must_use]
    pub const fn box_error(self) -> Option<isobmff_core::Error> {
        self.representation.fields().box_error
    }

    /// Returns the type of the box the failure names, for the kinds that name one
    #[must_use]
    pub const fn box_type(self) -> Option<BoxType> {
        self.representation.fields().box_type
    }

    /// Returns the bytes the failure required, for the kinds that count bytes
    #[must_use]
    pub const fn needed_bytes(self) -> Option<u64> {
        self.representation.fields().needed_bytes
    }

    /// Returns the bytes the failure had to hand, for the kinds that count bytes
    #[must_use]
    pub const fn available_bytes(self) -> Option<u64> {
        self.representation.fields().available_bytes
    }
}

impl From<isobmff_core::Error> for StructureError {
    /// Carries the failure of one box through as it stands
    fn from(box_error: isobmff_core::Error) -> Self {
        Self {
            representation: Representation::Box(box_error),
        }
    }
}

impl fmt::Display for StructureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.representation {
            Representation::Box(box_error) => write!(formatter, "{box_error}"),
            Representation::MissingMandatoryBox { box_type } => {
                write!(formatter, "file carries no {box_type} box")
            }
            Representation::DuplicateBox { box_type } => {
                write!(formatter, "file carries a second {box_type} box")
            }
            Representation::BoxOutOfOrder { box_type } => write!(
                formatter,
                "file carries a {box_type} box out of the order its structure keeps"
            ),
            Representation::PayloadLimitExceeded {
                box_type,
                reached,
                limit,
            } => write!(
                formatter,
                "{box_type} box reaches {reached} payload bytes, past the {limit}-byte limit"
            ),
            Representation::NoBoxOpen => {
                formatter.write_str("payload, or the end of a box, came while no box was open")
            }
            Representation::BoxStillOpen { box_type } => {
                write!(
                    formatter,
                    "a box came while the {box_type} box was still open"
                )
            }
            Representation::AlreadyFinished => {
                formatter.write_str("file was declared over and takes nothing more")
            }
        }
    }
}

impl fmt::Debug for StructureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let values = self.representation.fields();
        let mut fields = formatter.debug_struct("StructureError");
        fields.field("kind", &self.kind());
        fields.field("category", &self.category());

        if let Some(box_error) = values.box_error {
            fields.field("box_error", &box_error);
        }
        if let Some(box_type) = values.box_type {
            fields.field("box_type", &box_type);
        }
        if let Some(needed) = values.needed_bytes {
            fields.field("needed_bytes", &needed);
        }
        if let Some(available) = values.available_bytes {
            fields.field("available_bytes", &available);
        }

        fields.finish()
    }
}

impl error::Error for StructureError {
    /// Returns the failure of one box carried through, when it holds one
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        if let Representation::Box(box_error) = &self.representation {
            Some(box_error)
        } else {
            None
        }
    }
}

/// What a failure of reading a file through the layers this crate holds is
///
/// The vocabulary is this crate's own: the boxes a structure is made of, the
/// order it keeps them in, and reading one of them whole name their failures
/// here. A failure of one box is not translated: it keeps the kind
/// [`isobmff_core::ErrorKind`] gives it, carried on [`Box`](Self::Box).
///
/// The situations a structure reaches are added to as ISO/IEC 14496-12 is
/// read further, so a match on this must leave room for kinds that are not
/// here yet.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum StructureErrorKind {
    /// Failure of one box, carried through as `isobmff-core` names it
    ///
    /// The values that failure carries, and the boxes it was reached through,
    /// are on [`box_error`](StructureError::box_error).
    Box(isobmff_core::ErrorKind),
    /// Box the structure requires is not there
    ///
    /// The file was declared over without it.
    /// [`box_type`](StructureError::box_type) is the box that is missing.
    MissingMandatoryBox,
    /// Box the structure carries once is there twice
    ///
    /// [`box_type`](StructureError::box_type) is the box that came again.
    DuplicateBox,
    /// Box lies out of the order the structure keeps
    ///
    /// It came before a box the structure places ahead of it, or after one it
    /// places behind it. [`box_type`](StructureError::box_type) is the box
    /// that came out of order.
    BoxOutOfOrder,
    /// Box read whole reaches past the limit the reader gathers
    ///
    /// [`box_type`](StructureError::box_type) is the box,
    /// [`needed_bytes`](StructureError::needed_bytes) the payload it
    /// declares — or, for a box declaring no total, the payload it has
    /// reached — and [`available_bytes`](StructureError::available_bytes)
    /// the payload the reader gathers for one box at most.
    PayloadLimitExceeded,
    /// Payload, or the end of a box, came while no box was open
    NoBoxOpen,
    /// Box started while the box before it was still open
    ///
    /// [`box_type`](StructureError::box_type) is the box left open.
    BoxStillOpen,
    /// File was declared over, and takes nothing more
    AlreadyFinished,
}

/// Values a failure carries, keyed by what went wrong
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Representation {
    /// Failure of one box, carried through whole
    Box(isobmff_core::Error),
    /// Box the structure requires that is not there
    MissingMandatoryBox { box_type: BoxType },
    /// Box the structure carries once that is there twice
    DuplicateBox { box_type: BoxType },
    /// Box lying out of the order the structure keeps
    BoxOutOfOrder { box_type: BoxType },
    /// Box reaching past the limit a reader gathers
    PayloadLimitExceeded {
        box_type: BoxType,
        reached: u64,
        limit: u64,
    },
    /// Payload, or the end of a box, handed over while no box was open
    NoBoxOpen,
    /// Box handed over while the box before it was still open
    BoxStillOpen { box_type: BoxType },
    /// Call made after the file was declared over
    AlreadyFinished,
}

/// Values a failure carries, laid flat, with `None` where its kind carries no such value
struct Fields {
    box_error: Option<isobmff_core::Error>,
    box_type: Option<BoxType>,
    needed_bytes: Option<u64>,
    available_bytes: Option<u64>,
}

impl Fields {
    /// Values of a failure that carries none
    const EMPTY: Self = Self {
        box_error: None,
        box_type: None,
        needed_bytes: None,
        available_bytes: None,
    };
}

impl Representation {
    /// Returns the values the failure carries, laid flat
    const fn fields(self) -> Fields {
        match self {
            Self::Box(box_error) => Fields {
                box_error: Some(box_error),
                ..Fields::EMPTY
            },
            Self::MissingMandatoryBox { box_type }
            | Self::DuplicateBox { box_type }
            | Self::BoxOutOfOrder { box_type }
            | Self::BoxStillOpen { box_type } => Fields {
                box_type: Some(box_type),
                ..Fields::EMPTY
            },
            Self::PayloadLimitExceeded {
                box_type,
                reached,
                limit,
            } => Fields {
                box_type: Some(box_type),
                needed_bytes: Some(reached),
                available_bytes: Some(limit),
                ..Fields::EMPTY
            },
            Self::NoBoxOpen | Self::AlreadyFinished => Fields::EMPTY,
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::format;
    use alloc::string::ToString as _;

    use isobmff_core::{BoxType, Category};

    use super::{StructureError, StructureErrorKind};

    /// The `moov` box, which most of the structure's failures name
    const MOOV: BoxType = BoxType::compact(*b"moov");

    #[test]
    fn a_kind_falls_in_the_category_its_situation_asks_for() {
        assert_eq!(
            StructureError::missing_mandatory_box(MOOV).category(),
            Category::Malformed
        );
        assert_eq!(
            StructureError::payload_limit_exceeded(MOOV, 32, 16).category(),
            Category::Unsupported
        );
        assert_eq!(StructureError::no_box_open().category(), Category::Usage);
        assert_eq!(
            StructureError::from(isobmff_core::Error::unsupported_version(2)).category(),
            Category::Unsupported
        );
    }

    #[test]
    fn a_failure_carries_only_the_values_its_kind_names() {
        let missing = StructureError::missing_mandatory_box(MOOV);

        assert_eq!(missing.kind(), StructureErrorKind::MissingMandatoryBox);
        assert_eq!(missing.box_type(), Some(MOOV));
        assert_eq!(missing.needed_bytes(), None);
        assert_eq!(missing.box_error(), None);

        let exceeded = StructureError::payload_limit_exceeded(MOOV, 32, 16);

        assert_eq!(exceeded.kind(), StructureErrorKind::PayloadLimitExceeded);
        assert_eq!(exceeded.box_type(), Some(MOOV));
        assert_eq!(exceeded.needed_bytes(), Some(32));
        assert_eq!(exceeded.available_bytes(), Some(16));

        assert_eq!(StructureError::no_box_open().box_type(), None);
        assert_eq!(StructureError::box_still_open(MOOV).box_type(), Some(MOOV));
    }

    #[test]
    fn a_failure_of_one_box_keeps_its_values_and_the_boxes_it_was_reached_through() {
        let box_error = isobmff_core::Error::missing_mandatory_box(BoxType::compact(*b"trex"))
            .in_container(BoxType::compact(*b"mvex"));
        let carried = StructureError::from(box_error);

        assert_eq!(
            carried.kind(),
            StructureErrorKind::Box(isobmff_core::ErrorKind::MissingMandatoryBox)
        );
        assert_eq!(carried.box_error(), Some(box_error));
        assert_eq!(carried.box_type(), None);
        assert_eq!(carried.needed_bytes(), None);
    }

    #[test]
    fn display_of_a_failure_of_the_structure_states_the_reason() {
        assert_eq!(
            StructureError::missing_mandatory_box(MOOV).to_string(),
            "file carries no moov box"
        );
        assert_eq!(
            StructureError::duplicate_box(MOOV).to_string(),
            "file carries a second moov box"
        );
        assert_eq!(
            StructureError::box_out_of_order(MOOV).to_string(),
            "file carries a moov box out of the order its structure keeps"
        );
        assert_eq!(
            StructureError::payload_limit_exceeded(MOOV, 32, 16).to_string(),
            "moov box reaches 32 payload bytes, past the 16-byte limit"
        );
        assert_eq!(
            StructureError::no_box_open().to_string(),
            "payload, or the end of a box, came while no box was open"
        );
        assert_eq!(
            StructureError::box_still_open(MOOV).to_string(),
            "a box came while the moov box was still open"
        );
        assert_eq!(
            StructureError::already_finished().to_string(),
            "file was declared over and takes nothing more"
        );
    }

    #[test]
    fn display_of_a_failure_of_one_box_reads_as_that_failure() {
        let box_error = isobmff_core::Error::unsupported_version(2);

        assert_eq!(
            StructureError::from(box_error).to_string(),
            box_error.to_string()
        );
    }

    #[test]
    fn debug_names_the_values_a_kind_carries_and_leaves_out_the_rest() {
        assert_eq!(
            format!("{:?}", StructureError::payload_limit_exceeded(MOOV, 32, 16)),
            "StructureError { kind: PayloadLimitExceeded, category: Unsupported, box_type: Compact(CompactType(FourCC(\"moov\"))), needed_bytes: 32, available_bytes: 16 }"
        );
        assert_eq!(
            format!("{:?}", StructureError::no_box_open()),
            "StructureError { kind: NoBoxOpen, category: Usage }"
        );
    }
}
