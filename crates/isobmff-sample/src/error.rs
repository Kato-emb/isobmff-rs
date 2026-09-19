//! [`SampleError`], the reason the samples of a presentation do not resolve

use core::error;
use core::fmt;

use isobmff_core::Category;

/// Reason the samples of a presentation do not resolve
///
/// What went wrong is one [`kind`](Self::kind): a failure of the samples
/// themselves — a timeline run past what its field carries — or a failure of
/// one box, which [`isobmff_core::Error`] names and this type carries through
/// whole, as [`box_error`](Self::box_error). What a caller does about either is
/// one [`category`](Self::category).
///
/// The values a failure of the samples carries follow from its kind, and each
/// kind names its own on [`SampleErrorKind`]. A carried box failure keeps its
/// values and its container path on [`box_error`](Self::box_error), so the
/// accessors here report `None` for it.
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxType, Category};
/// use isobmff_sample::{SampleError, SampleErrorKind};
///
/// // A failure of the samples names its own kind
/// let failure = SampleError::decode_time_overflow(3);
/// assert_eq!(failure.kind(), SampleErrorKind::DecodeTimeOverflow);
/// assert_eq!(failure.category(), Category::Malformed);
/// assert_eq!(failure.track_id(), Some(3));
/// assert_eq!(failure.box_error(), None);
///
/// // A failure of one box is carried through whole
/// let missing = isobmff_core::Error::missing_mandatory_box(BoxType::compact(*b"trex"));
/// let carried = SampleError::from(missing);
/// assert_eq!(
///     carried.kind(),
///     SampleErrorKind::Box(isobmff_core::ErrorKind::MissingMandatoryBox)
/// );
/// assert_eq!(
///     carried.box_error().and_then(|box_error| box_error.box_type()),
///     Some(BoxType::compact(*b"trex"))
/// );
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SampleError {
    representation: Representation,
}

impl SampleError {
    /// Returns the failure of a decode time running past what 64 bits carry
    #[must_use]
    pub const fn decode_time_overflow(track_id: u32) -> Self {
        Self {
            representation: Representation::DecodeTimeOverflow { track_id },
        }
    }

    /// Returns what went wrong
    #[must_use]
    pub const fn kind(self) -> SampleErrorKind {
        match self.representation {
            Representation::Box(box_error) => SampleErrorKind::Box(box_error.kind()),
            Representation::DecodeTimeOverflow { .. } => SampleErrorKind::DecodeTimeOverflow,
        }
    }

    /// Returns what a caller does about the failure
    #[must_use]
    pub const fn category(self) -> Category {
        match self.representation {
            Representation::Box(box_error) => box_error.category(),
            Representation::DecodeTimeOverflow { .. } => Category::Malformed,
        }
    }

    /// Returns the failure of one box carried through, when it holds one
    ///
    /// The values that failure carries, and the boxes it was reached through,
    /// are read off the [`isobmff_core::Error`] itself.
    #[must_use]
    pub const fn box_error(self) -> Option<isobmff_core::Error> {
        match self.representation {
            Representation::Box(box_error) => Some(box_error),
            Representation::DecodeTimeOverflow { .. } => None,
        }
    }

    /// Returns the track the failure is about, for the kinds that name one
    #[must_use]
    pub const fn track_id(self) -> Option<u32> {
        match self.representation {
            Representation::DecodeTimeOverflow { track_id } => Some(track_id),
            Representation::Box(_) => None,
        }
    }
}

impl From<isobmff_core::Error> for SampleError {
    /// Carries the failure of one box through as it stands
    fn from(box_error: isobmff_core::Error) -> Self {
        Self {
            representation: Representation::Box(box_error),
        }
    }
}

impl fmt::Display for SampleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.representation {
            Representation::Box(box_error) => write!(formatter, "{box_error}"),
            Representation::DecodeTimeOverflow { track_id } => write!(
                formatter,
                "decode time of track {track_id} runs past what 64 bits carry"
            ),
        }
    }
}

impl fmt::Debug for SampleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut fields = formatter.debug_struct("SampleError");
        fields.field("kind", &self.kind());
        fields.field("category", &self.category());

        if let Some(box_error) = self.box_error() {
            fields.field("box_error", &box_error);
        }
        if let Some(track_id) = self.track_id() {
            fields.field("track_id", &track_id);
        }

        fields.finish()
    }
}

impl error::Error for SampleError {
    /// Returns the failure of one box carried through, when it holds one
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.representation {
            Representation::Box(box_error) => Some(box_error),
            Representation::DecodeTimeOverflow { .. } => None,
        }
    }
}

/// What a failure of the samples of a presentation is
///
/// The vocabulary is this crate's own: resolving where a sample lies, gathering
/// it, and laying it out in a declaration name their failures here. A failure
/// of one box is not translated: it keeps the kind [`isobmff_core::ErrorKind`]
/// gives it, carried on [`Box`](Self::Box).
///
/// The situations this layer reaches are added to as ISO/IEC 14496-12 is read
/// further, so a match on this must leave room for kinds that are not here yet.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SampleErrorKind {
    /// Failure of one box, carried through as `isobmff-core` names it
    ///
    /// The values that failure carries, and the boxes it was reached through,
    /// are on [`box_error`](SampleError::box_error).
    Box(isobmff_core::ErrorKind),
    /// Decode times of a track run past what 64 bits carry
    ///
    /// [`track_id`](SampleError::track_id) is the track they belong to.
    DecodeTimeOverflow,
}

/// Values a failure carries, keyed by what went wrong
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Representation {
    /// Failure of one box, carried through whole
    Box(isobmff_core::Error),
    /// Decode time running past what 64 bits carry
    DecodeTimeOverflow { track_id: u32 },
}

#[cfg(test)]
mod tests {
    use alloc::format;
    use alloc::string::ToString as _;

    use isobmff_core::{BoxType, Category};

    use super::SampleError;

    #[test]
    fn a_kind_falls_in_the_category_its_situation_asks_for() {
        assert_eq!(
            SampleError::decode_time_overflow(3).category(),
            Category::Malformed
        );
        assert_eq!(
            SampleError::from(isobmff_core::Error::unsupported_version(2)).category(),
            Category::Unsupported
        );
    }

    #[test]
    fn a_failure_of_one_box_keeps_its_values_and_the_boxes_it_was_reached_through() {
        let box_error = isobmff_core::Error::missing_mandatory_box(BoxType::compact(*b"trex"))
            .in_container(BoxType::compact(*b"mvex"));
        let carried = SampleError::from(box_error);

        assert_eq!(carried.box_error(), Some(box_error));
        assert_eq!(carried.track_id(), None);
    }

    #[test]
    fn display_of_a_failure_of_the_samples_states_the_reason() {
        assert_eq!(
            SampleError::decode_time_overflow(1).to_string(),
            "decode time of track 1 runs past what 64 bits carry"
        );
    }

    #[test]
    fn display_of_a_failure_of_one_box_reads_as_that_failure() {
        let box_error = isobmff_core::Error::missing_mandatory_box(BoxType::compact(*b"mvex"));

        assert_eq!(
            SampleError::from(box_error).to_string(),
            box_error.to_string()
        );
    }

    #[test]
    fn debug_names_the_values_a_kind_carries_and_leaves_out_the_rest() {
        assert_eq!(
            format!("{:?}", SampleError::decode_time_overflow(1)),
            "SampleError { kind: DecodeTimeOverflow, category: Malformed, track_id: 1 }"
        );
    }
}
