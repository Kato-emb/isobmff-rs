//! [`SampleError`], the reason the samples of a presentation do not resolve

use core::error;
use core::fmt;

use isobmff_core::Category;

mod kind;

use crate::error::kind::Representation;
pub use kind::SampleErrorKind;

/// Reason the samples of a presentation do not resolve
///
/// What went wrong is one [`kind`](Self::kind): a failure of the samples
/// themselves — a fragment of a track the movie never declared, a timeline or
/// an offset run past what its field carries, a sample that never arrived
/// whole — or a failure of one box, which
/// [`isobmff_core::Error`] names and this type carries through whole, as
/// [`box_error`](Self::box_error). What a caller does about either is one
/// [`category`](Self::category).
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

    /// Returns the failure of a data offset running past what 64 bits carry
    #[must_use]
    pub const fn data_offset_overflow(track_id: u32) -> Self {
        Self {
            representation: Representation::DataOffsetOverflow { track_id },
        }
    }

    /// Returns the failure of a fragment carrying samples of a track the movie never declared
    #[must_use]
    pub const fn unknown_track_id(track_id: u32) -> Self {
        Self {
            representation: Representation::UnknownTrackId { track_id },
        }
    }

    /// Returns the failure of samples described by an `stsd` entry their track has none of
    #[must_use]
    pub const fn unknown_sample_description_index(
        track_id: u32,
        sample_description_index: u32,
    ) -> Self {
        Self {
            representation: Representation::UnknownSampleDescriptionIndex {
                track_id,
                sample_description_index,
            },
        }
    }

    /// Returns the failure of a movie that carries no `mvex`, and so no fragments
    #[must_use]
    pub const fn missing_movie_extends() -> Self {
        Self {
            representation: Representation::MissingMovieExtends,
        }
    }

    /// Returns the failure of an `stsd` entry naming a `dref` entry the track has none of
    #[must_use]
    pub const fn unknown_data_reference_index(track_id: u32, data_reference_index: u16) -> Self {
        Self {
            representation: Representation::UnknownDataReferenceIndex {
                track_id,
                data_reference_index,
            },
        }
    }

    /// Returns the failure of a `dref` entry naming a resource other than the file itself
    #[must_use]
    pub const fn external_data_reference(track_id: u32, data_reference_index: u16) -> Self {
        Self {
            representation: Representation::ExternalDataReference {
                track_id,
                data_reference_index,
            },
        }
    }

    /// Returns the failure of the sample tables of a track counting different numbers of samples
    #[must_use]
    pub const fn sample_count_mismatch(track_id: u32) -> Self {
        Self {
            representation: Representation::SampleCountMismatch { track_id },
        }
    }

    /// Returns the failure of a run of chunks starting at a chunk outside the range open to it
    #[must_use]
    pub const fn first_chunk_out_of_range(track_id: u32, first_chunk: u32) -> Self {
        Self {
            representation: Representation::FirstChunkOutOfRange {
                track_id,
                first_chunk,
            },
        }
    }

    /// Returns the failure of a sample declared past the limit a reader holds
    #[must_use]
    pub const fn sample_size_limit_exceeded(track_id: u32, declared: u64, limit: u64) -> Self {
        Self {
            representation: Representation::SampleSizeLimitExceeded {
                track_id,
                declared,
                limit,
            },
        }
    }

    /// Returns the failure of a sample whose bytes never arrived whole
    #[must_use]
    pub const fn unfinished_sample(track_id: u32, needed: u64, available: u64) -> Self {
        Self {
            representation: Representation::UnfinishedSample {
                track_id,
                needed,
                available,
            },
        }
    }

    /// Returns the failure of a call made after the samples were declared over
    #[must_use]
    pub const fn already_finished() -> Self {
        Self {
            representation: Representation::AlreadyFinished,
        }
    }

    /// Returns what went wrong
    #[must_use]
    pub const fn kind(self) -> SampleErrorKind {
        self.representation.kind()
    }

    /// Returns what a caller does about the failure
    #[must_use]
    pub const fn category(self) -> Category {
        self.representation.category()
    }

    /// Returns the failure of one box carried through, when it holds one
    ///
    /// The values that failure carries, and the boxes it was reached through,
    /// are read off the [`isobmff_core::Error`] itself.
    #[must_use]
    pub const fn box_error(self) -> Option<isobmff_core::Error> {
        match self.representation {
            Representation::Box(box_error) => Some(box_error),
            Representation::DecodeTimeOverflow { .. }
            | Representation::DataOffsetOverflow { .. }
            | Representation::UnknownTrackId { .. }
            | Representation::UnknownSampleDescriptionIndex { .. }
            | Representation::MissingMovieExtends
            | Representation::UnknownDataReferenceIndex { .. }
            | Representation::ExternalDataReference { .. }
            | Representation::SampleCountMismatch { .. }
            | Representation::FirstChunkOutOfRange { .. }
            | Representation::SampleSizeLimitExceeded { .. }
            | Representation::UnfinishedSample { .. }
            | Representation::AlreadyFinished => None,
        }
    }

    /// Returns the track the failure is about, for the kinds that name one
    #[must_use]
    pub const fn track_id(self) -> Option<u32> {
        match self.representation {
            Representation::DecodeTimeOverflow { track_id }
            | Representation::DataOffsetOverflow { track_id }
            | Representation::UnknownTrackId { track_id }
            | Representation::UnknownSampleDescriptionIndex { track_id, .. }
            | Representation::UnknownDataReferenceIndex { track_id, .. }
            | Representation::ExternalDataReference { track_id, .. }
            | Representation::SampleCountMismatch { track_id }
            | Representation::FirstChunkOutOfRange { track_id, .. }
            | Representation::SampleSizeLimitExceeded { track_id, .. }
            | Representation::UnfinishedSample { track_id, .. } => Some(track_id),
            Representation::Box(_)
            | Representation::MissingMovieExtends
            | Representation::AlreadyFinished => None,
        }
    }

    /// Returns the `stsd` entry the failure names, for the kinds that name one
    #[must_use]
    pub const fn sample_description_index(self) -> Option<u32> {
        match self.representation {
            Representation::UnknownSampleDescriptionIndex {
                sample_description_index,
                ..
            } => Some(sample_description_index),
            Representation::Box(_)
            | Representation::DecodeTimeOverflow { .. }
            | Representation::DataOffsetOverflow { .. }
            | Representation::UnknownTrackId { .. }
            | Representation::MissingMovieExtends
            | Representation::UnknownDataReferenceIndex { .. }
            | Representation::ExternalDataReference { .. }
            | Representation::SampleCountMismatch { .. }
            | Representation::FirstChunkOutOfRange { .. }
            | Representation::SampleSizeLimitExceeded { .. }
            | Representation::UnfinishedSample { .. }
            | Representation::AlreadyFinished => None,
        }
    }

    /// Returns the `dref` entry the failure names, for the kinds that name one
    #[must_use]
    pub const fn data_reference_index(self) -> Option<u16> {
        match self.representation {
            Representation::UnknownDataReferenceIndex {
                data_reference_index,
                ..
            }
            | Representation::ExternalDataReference {
                data_reference_index,
                ..
            } => Some(data_reference_index),
            Representation::Box(_)
            | Representation::DecodeTimeOverflow { .. }
            | Representation::DataOffsetOverflow { .. }
            | Representation::UnknownTrackId { .. }
            | Representation::UnknownSampleDescriptionIndex { .. }
            | Representation::MissingMovieExtends
            | Representation::SampleCountMismatch { .. }
            | Representation::FirstChunkOutOfRange { .. }
            | Representation::SampleSizeLimitExceeded { .. }
            | Representation::UnfinishedSample { .. }
            | Representation::AlreadyFinished => None,
        }
    }

    /// Returns the chunk a run of chunks starts at, counted from one, for the kinds that name one
    #[must_use]
    pub const fn first_chunk(self) -> Option<u32> {
        match self.representation {
            Representation::FirstChunkOutOfRange { first_chunk, .. } => Some(first_chunk),
            Representation::Box(_)
            | Representation::DecodeTimeOverflow { .. }
            | Representation::DataOffsetOverflow { .. }
            | Representation::UnknownTrackId { .. }
            | Representation::UnknownSampleDescriptionIndex { .. }
            | Representation::MissingMovieExtends
            | Representation::UnknownDataReferenceIndex { .. }
            | Representation::ExternalDataReference { .. }
            | Representation::SampleCountMismatch { .. }
            | Representation::SampleSizeLimitExceeded { .. }
            | Representation::UnfinishedSample { .. }
            | Representation::AlreadyFinished => None,
        }
    }

    /// Returns the bytes the failure required, for the kinds that count bytes
    #[must_use]
    pub const fn needed_bytes(self) -> Option<u64> {
        match self.representation {
            Representation::SampleSizeLimitExceeded { declared, .. } => Some(declared),
            Representation::UnfinishedSample { needed, .. } => Some(needed),
            Representation::Box(_)
            | Representation::DecodeTimeOverflow { .. }
            | Representation::DataOffsetOverflow { .. }
            | Representation::UnknownTrackId { .. }
            | Representation::UnknownSampleDescriptionIndex { .. }
            | Representation::MissingMovieExtends
            | Representation::UnknownDataReferenceIndex { .. }
            | Representation::ExternalDataReference { .. }
            | Representation::SampleCountMismatch { .. }
            | Representation::FirstChunkOutOfRange { .. }
            | Representation::AlreadyFinished => None,
        }
    }

    /// Returns the bytes the failure had to hand, for the kinds that count bytes
    #[must_use]
    pub const fn available_bytes(self) -> Option<u64> {
        match self.representation {
            Representation::SampleSizeLimitExceeded { limit, .. } => Some(limit),
            Representation::UnfinishedSample { available, .. } => Some(available),
            Representation::Box(_)
            | Representation::DecodeTimeOverflow { .. }
            | Representation::DataOffsetOverflow { .. }
            | Representation::UnknownTrackId { .. }
            | Representation::UnknownSampleDescriptionIndex { .. }
            | Representation::MissingMovieExtends
            | Representation::UnknownDataReferenceIndex { .. }
            | Representation::ExternalDataReference { .. }
            | Representation::SampleCountMismatch { .. }
            | Representation::FirstChunkOutOfRange { .. }
            | Representation::AlreadyFinished => None,
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
            Representation::DataOffsetOverflow { track_id } => write!(
                formatter,
                "data offset of track {track_id} runs past what 64 bits carry"
            ),
            Representation::UnknownTrackId { track_id } => {
                write!(formatter, "movie declares no track {track_id}")
            }
            Representation::UnknownSampleDescriptionIndex {
                track_id,
                sample_description_index,
            } => write!(
                formatter,
                "track {track_id} has no stsd entry {sample_description_index}"
            ),
            Representation::MissingMovieExtends => formatter.write_str("the movie carries no mvex"),
            Representation::UnknownDataReferenceIndex {
                track_id,
                data_reference_index,
            } => write!(
                formatter,
                "track {track_id} has no dref entry {data_reference_index}"
            ),
            Representation::ExternalDataReference {
                track_id,
                data_reference_index,
            } => write!(
                formatter,
                "dref entry {data_reference_index} of track {track_id} names an external file"
            ),
            Representation::SampleCountMismatch { track_id } => write!(
                formatter,
                "sample tables of track {track_id} count different numbers of samples"
            ),
            Representation::FirstChunkOutOfRange {
                track_id,
                first_chunk,
            } => write!(
                formatter,
                "run of chunks of track {track_id} starts at chunk {first_chunk}, out of the range open to it"
            ),
            Representation::SampleSizeLimitExceeded {
                track_id,
                declared,
                limit,
            } => write!(
                formatter,
                "track {track_id} declares a sample of {declared} bytes, past the {limit}-byte limit"
            ),
            Representation::UnfinishedSample {
                track_id,
                needed,
                available,
            } => write!(
                formatter,
                "sample of track {track_id} takes {needed} bytes, and {available} arrived"
            ),
            Representation::AlreadyFinished => {
                formatter.write_str("samples were declared over and take nothing more")
            }
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
        if let Some(sample_description_index) = self.sample_description_index() {
            fields.field("sample_description_index", &sample_description_index);
        }
        if let Some(data_reference_index) = self.data_reference_index() {
            fields.field("data_reference_index", &data_reference_index);
        }
        if let Some(first_chunk) = self.first_chunk() {
            fields.field("first_chunk", &first_chunk);
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

impl error::Error for SampleError {
    /// Returns the failure of one box carried through, when it holds one
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.representation {
            Representation::Box(box_error) => Some(box_error),
            Representation::DecodeTimeOverflow { .. }
            | Representation::DataOffsetOverflow { .. }
            | Representation::UnknownTrackId { .. }
            | Representation::UnknownSampleDescriptionIndex { .. }
            | Representation::MissingMovieExtends
            | Representation::UnknownDataReferenceIndex { .. }
            | Representation::ExternalDataReference { .. }
            | Representation::SampleCountMismatch { .. }
            | Representation::FirstChunkOutOfRange { .. }
            | Representation::SampleSizeLimitExceeded { .. }
            | Representation::UnfinishedSample { .. }
            | Representation::AlreadyFinished => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::format;
    use alloc::string::ToString as _;

    use isobmff_core::BoxType;

    use super::SampleError;

    #[test]
    fn a_failure_carries_only_the_values_its_kind_names() {
        let error = SampleError::unknown_sample_description_index(2, 7);

        assert_eq!(error.track_id(), Some(2));
        assert_eq!(error.sample_description_index(), Some(7));
        assert_eq!(error.box_error(), None);
        assert_eq!(error.needed_bytes(), None);

        let unfinished = SampleError::unfinished_sample(2, 1_024, 512);

        assert_eq!(unfinished.needed_bytes(), Some(1_024));
        assert_eq!(unfinished.available_bytes(), Some(512));
        assert_eq!(unfinished.sample_description_index(), None);
        assert_eq!(SampleError::missing_movie_extends().track_id(), None);
        assert_eq!(
            SampleError::unknown_track_id(3).sample_description_index(),
            None
        );

        let external = SampleError::external_data_reference(1, 2);

        assert_eq!(external.track_id(), Some(1));
        assert_eq!(external.data_reference_index(), Some(2));
        assert_eq!(external.first_chunk(), None);
        assert_eq!(
            SampleError::first_chunk_out_of_range(1, 3).first_chunk(),
            Some(3)
        );
        assert_eq!(
            SampleError::sample_count_mismatch(1).data_reference_index(),
            None
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
        assert_eq!(
            SampleError::data_offset_overflow(1).to_string(),
            "data offset of track 1 runs past what 64 bits carry"
        );
        assert_eq!(
            SampleError::unknown_track_id(3).to_string(),
            "movie declares no track 3"
        );
        assert_eq!(
            SampleError::unknown_sample_description_index(2, 7).to_string(),
            "track 2 has no stsd entry 7"
        );
        assert_eq!(
            SampleError::missing_movie_extends().to_string(),
            "the movie carries no mvex"
        );
        assert_eq!(
            SampleError::unknown_data_reference_index(2, 3).to_string(),
            "track 2 has no dref entry 3"
        );
        assert_eq!(
            SampleError::external_data_reference(2, 3).to_string(),
            "dref entry 3 of track 2 names an external file"
        );
        assert_eq!(
            SampleError::sample_count_mismatch(2).to_string(),
            "sample tables of track 2 count different numbers of samples"
        );
        assert_eq!(
            SampleError::first_chunk_out_of_range(2, 5).to_string(),
            "run of chunks of track 2 starts at chunk 5, out of the range open to it"
        );
        assert_eq!(
            SampleError::sample_size_limit_exceeded(1, 32, 16).to_string(),
            "track 1 declares a sample of 32 bytes, past the 16-byte limit"
        );
        assert_eq!(
            SampleError::unfinished_sample(2, 1_024, 512).to_string(),
            "sample of track 2 takes 1024 bytes, and 512 arrived"
        );
        assert_eq!(
            SampleError::already_finished().to_string(),
            "samples were declared over and take nothing more"
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
        assert_eq!(
            format!("{:?}", SampleError::missing_movie_extends()),
            "SampleError { kind: MissingMovieExtends, category: Malformed }"
        );
        assert_eq!(
            format!("{:?}", SampleError::sample_size_limit_exceeded(1, 32, 16)),
            "SampleError { kind: SampleSizeLimitExceeded, category: Unsupported, track_id: 1, needed_bytes: 32, available_bytes: 16 }"
        );
        assert_eq!(
            format!("{:?}", SampleError::external_data_reference(1, 2)),
            "SampleError { kind: ExternalDataReference, category: Unsupported, track_id: 1, data_reference_index: 2 }"
        );
        assert_eq!(
            format!("{:?}", SampleError::first_chunk_out_of_range(1, 5)),
            "SampleError { kind: FirstChunkOutOfRange, category: Malformed, track_id: 1, first_chunk: 5 }"
        );
    }
}
