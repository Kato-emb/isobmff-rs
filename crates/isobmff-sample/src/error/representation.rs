//! [`Representation`], the values a failure of the samples carries, keyed by what went wrong

use isobmff_core::Category;

use crate::error::kind::ErrorKind;

/// Values a failure carries, keyed by what went wrong
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Representation {
    /// Failure of one box, carried through whole
    Box(isobmff_core::Error),
    /// Decode time running past what 64 bits carry
    DecodeTimeOverflow { track_id: u32 },
    /// Data offset running past what 64 bits carry
    DataOffsetOverflow { track_id: u32 },
    /// Fragment carrying samples of a track the movie never declared
    UnknownTrackId { track_id: u32 },
    /// Samples described by an `stsd` entry their track has none of
    UnknownSampleDescriptionIndex {
        track_id: u32,
        sample_description_index: u32,
    },
    /// Movie carrying no `mvex`, and so no fragments
    MissingMovieExtends,
    /// Sample entry naming a `dref` entry the track has none of
    UnknownDataReferenceIndex {
        track_id: u32,
        data_reference_index: u16,
    },
    /// Data reference naming a resource other than the file itself
    ExternalDataReference {
        track_id: u32,
        data_reference_index: u16,
    },
    /// Sample tables of a track counting different numbers of samples
    SampleCountMismatch { track_id: u32 },
    /// Run of chunks starting at a chunk outside the range open to it
    FirstChunkOutOfRange { track_id: u32, first_chunk: u32 },
    SyncSampleOutOfRange { track_id: u32, sample_number: u32 },
    /// Sample declared past the limit a reader holds
    SampleSizeLimitExceeded {
        track_id: u32,
        declared: u64,
        limit: u64,
    },
    /// Sample whose bytes never arrived whole
    UnfinishedSample {
        track_id: u32,
        needed: u64,
        available: u64,
    },
    /// Call made after the samples were declared over
    AlreadyFinished,
    /// Sample handed over, or a fragment closed, while no fragment was open
    NoFragmentOpen,
    /// Fragment begun while the one before it was still open
    FragmentStillOpen,
    /// Sample longer than the field a `trun` row or an `stsz` entry states its length in reaches
    SampleSizeOutOfRange { track_id: u32, declared: u64 },
    /// Sample lying further into its fragment than the offset a `trun` states reaches
    DataOffsetOutOfRange { track_id: u32, offset: u64 },
    /// Sample stating a composition time offset neither version of a `trun` writes
    CompositionTimeOffsetOutOfRange { track_id: u32, offset: i64 },
    /// Sample not starting where the one before it in its track ends
    DecodeTimeMismatch {
        track_id: u32,
        stated: u64,
        reached: u64,
    },
    /// Fragment of a track starting before the samples written for it reach
    BackwardDecodeTime {
        track_id: u32,
        stated: u64,
        reached: u64,
    },
    /// Samples of one fragment of one track, or of one chunk, described by two `stsd` entries
    SampleDescriptionIndexMismatch {
        track_id: u32,
        stated: u32,
        established: u32,
    },
    /// Sample handed over while no chunk was open
    NoChunkOpen,
    /// Sample belonging to another track than the chunk that is open holds
    TrackIdMismatch { stated: u32, established: u32 },
    /// Sample stating a composition time offset no sample table written here carries
    UnsupportedCompositionTimeOffset { track_id: u32, offset: i64 },
    /// Sample stating flags no sample table written here carries
    UnsupportedSampleFlags { track_id: u32, sample_flags: u32 },
}

/// Values a failure carries, laid flat, with `None` where its kind carries no such value
pub(super) struct Fields {
    pub(super) box_error: Option<isobmff_core::Error>,
    pub(super) track_id: Option<u32>,
    pub(super) established_track_id: Option<u32>,
    pub(super) sample_description_index: Option<u32>,
    pub(super) established_sample_description_index: Option<u32>,
    pub(super) data_reference_index: Option<u16>,
    pub(super) first_chunk: Option<u32>,
    pub(super) sample_number: Option<u32>,
    pub(super) needed_bytes: Option<u64>,
    pub(super) available_bytes: Option<u64>,
    pub(super) stated_decode_time: Option<u64>,
    pub(super) reached_decode_time: Option<u64>,
    pub(super) data_offset: Option<u64>,
    pub(super) composition_time_offset: Option<i64>,
    pub(super) sample_flags: Option<u32>,
}

impl Fields {
    /// Values of a failure that carries none
    const EMPTY: Self = Self {
        box_error: None,
        track_id: None,
        established_track_id: None,
        sample_description_index: None,
        established_sample_description_index: None,
        data_reference_index: None,
        first_chunk: None,
        sample_number: None,
        needed_bytes: None,
        available_bytes: None,
        stated_decode_time: None,
        reached_decode_time: None,
        data_offset: None,
        composition_time_offset: None,
        sample_flags: None,
    };
}

impl Representation {
    /// Returns what went wrong
    pub(super) const fn kind(self) -> ErrorKind {
        match self {
            Self::Box(box_error) => ErrorKind::Box(box_error.kind()),
            Self::DecodeTimeOverflow { .. } => ErrorKind::DecodeTimeOverflow,
            Self::DataOffsetOverflow { .. } => ErrorKind::DataOffsetOverflow,
            Self::UnknownTrackId { .. } => ErrorKind::UnknownTrackId,
            Self::UnknownSampleDescriptionIndex { .. } => ErrorKind::UnknownSampleDescriptionIndex,
            Self::MissingMovieExtends => ErrorKind::MissingMovieExtends,
            Self::UnknownDataReferenceIndex { .. } => ErrorKind::UnknownDataReferenceIndex,
            Self::ExternalDataReference { .. } => ErrorKind::ExternalDataReference,
            Self::SampleCountMismatch { .. } => ErrorKind::SampleCountMismatch,
            Self::FirstChunkOutOfRange { .. } => ErrorKind::FirstChunkOutOfRange,
            Self::SyncSampleOutOfRange { .. } => ErrorKind::SyncSampleOutOfRange,
            Self::SampleSizeLimitExceeded { .. } => ErrorKind::SampleSizeLimitExceeded,
            Self::UnfinishedSample { .. } => ErrorKind::UnfinishedSample,
            Self::AlreadyFinished => ErrorKind::AlreadyFinished,
            Self::NoFragmentOpen => ErrorKind::NoFragmentOpen,
            Self::FragmentStillOpen => ErrorKind::FragmentStillOpen,
            Self::SampleSizeOutOfRange { .. } => ErrorKind::SampleSizeOutOfRange,
            Self::DataOffsetOutOfRange { .. } => ErrorKind::DataOffsetOutOfRange,
            Self::CompositionTimeOffsetOutOfRange { .. } => {
                ErrorKind::CompositionTimeOffsetOutOfRange
            }
            Self::DecodeTimeMismatch { .. } => ErrorKind::DecodeTimeMismatch,
            Self::BackwardDecodeTime { .. } => ErrorKind::BackwardDecodeTime,
            Self::SampleDescriptionIndexMismatch { .. } => {
                ErrorKind::SampleDescriptionIndexMismatch
            }
            Self::NoChunkOpen => ErrorKind::NoChunkOpen,
            Self::TrackIdMismatch { .. } => ErrorKind::TrackIdMismatch,
            Self::UnsupportedCompositionTimeOffset { .. } => {
                ErrorKind::UnsupportedCompositionTimeOffset
            }
            Self::UnsupportedSampleFlags { .. } => ErrorKind::UnsupportedSampleFlags,
        }
    }

    /// Returns what a caller does about the failure
    pub(super) const fn category(self) -> Category {
        match self {
            Self::Box(box_error) => box_error.category(),
            Self::DecodeTimeOverflow { .. }
            | Self::DataOffsetOverflow { .. }
            | Self::UnknownTrackId { .. }
            | Self::UnknownSampleDescriptionIndex { .. }
            | Self::MissingMovieExtends
            | Self::UnknownDataReferenceIndex { .. }
            | Self::SampleCountMismatch { .. }
            | Self::FirstChunkOutOfRange { .. }
            | Self::SyncSampleOutOfRange { .. }
            | Self::UnfinishedSample { .. }
            | Self::DecodeTimeMismatch { .. }
            | Self::BackwardDecodeTime { .. }
            | Self::SampleDescriptionIndexMismatch { .. }
            | Self::TrackIdMismatch { .. } => Category::Malformed,
            Self::ExternalDataReference { .. }
            | Self::SampleSizeLimitExceeded { .. }
            | Self::SampleSizeOutOfRange { .. }
            | Self::DataOffsetOutOfRange { .. }
            | Self::CompositionTimeOffsetOutOfRange { .. }
            | Self::UnsupportedCompositionTimeOffset { .. }
            | Self::UnsupportedSampleFlags { .. } => Category::Unsupported,
            Self::AlreadyFinished
            | Self::NoFragmentOpen
            | Self::FragmentStillOpen
            | Self::NoChunkOpen => Category::Usage,
        }
    }

    /// Returns the values the failure carries, laid flat
    pub(super) const fn fields(self) -> Fields {
        match self {
            Self::Box(box_error) => Fields {
                box_error: Some(box_error),
                ..Fields::EMPTY
            },
            Self::DecodeTimeOverflow { track_id }
            | Self::DataOffsetOverflow { track_id }
            | Self::UnknownTrackId { track_id }
            | Self::SampleCountMismatch { track_id } => Fields {
                track_id: Some(track_id),
                ..Fields::EMPTY
            },
            Self::UnknownSampleDescriptionIndex {
                track_id,
                sample_description_index,
            } => Fields {
                track_id: Some(track_id),
                sample_description_index: Some(sample_description_index),
                ..Fields::EMPTY
            },
            Self::MissingMovieExtends
            | Self::AlreadyFinished
            | Self::NoFragmentOpen
            | Self::FragmentStillOpen
            | Self::NoChunkOpen => Fields::EMPTY,
            Self::UnknownDataReferenceIndex {
                track_id,
                data_reference_index,
            }
            | Self::ExternalDataReference {
                track_id,
                data_reference_index,
            } => Fields {
                track_id: Some(track_id),
                data_reference_index: Some(data_reference_index),
                ..Fields::EMPTY
            },
            Self::FirstChunkOutOfRange {
                track_id,
                first_chunk,
            } => Fields {
                track_id: Some(track_id),
                first_chunk: Some(first_chunk),
                ..Fields::EMPTY
            },
            Self::SyncSampleOutOfRange {
                track_id,
                sample_number,
            } => Fields {
                track_id: Some(track_id),
                sample_number: Some(sample_number),
                ..Fields::EMPTY
            },
            Self::SampleSizeLimitExceeded {
                track_id,
                declared,
                limit,
            } => Fields {
                track_id: Some(track_id),
                needed_bytes: Some(declared),
                available_bytes: Some(limit),
                ..Fields::EMPTY
            },
            Self::UnfinishedSample {
                track_id,
                needed,
                available,
            } => Fields {
                track_id: Some(track_id),
                needed_bytes: Some(needed),
                available_bytes: Some(available),
                ..Fields::EMPTY
            },
            Self::SampleSizeOutOfRange { track_id, declared } => Fields {
                track_id: Some(track_id),
                needed_bytes: Some(declared),
                ..Fields::EMPTY
            },
            Self::DataOffsetOutOfRange { track_id, offset } => Fields {
                track_id: Some(track_id),
                data_offset: Some(offset),
                ..Fields::EMPTY
            },
            Self::CompositionTimeOffsetOutOfRange { track_id, offset }
            | Self::UnsupportedCompositionTimeOffset { track_id, offset } => Fields {
                track_id: Some(track_id),
                composition_time_offset: Some(offset),
                ..Fields::EMPTY
            },
            Self::UnsupportedSampleFlags {
                track_id,
                sample_flags,
            } => Fields {
                track_id: Some(track_id),
                sample_flags: Some(sample_flags),
                ..Fields::EMPTY
            },
            Self::TrackIdMismatch {
                stated,
                established,
            } => Fields {
                track_id: Some(stated),
                established_track_id: Some(established),
                ..Fields::EMPTY
            },
            Self::DecodeTimeMismatch {
                track_id,
                stated,
                reached,
            }
            | Self::BackwardDecodeTime {
                track_id,
                stated,
                reached,
            } => Fields {
                track_id: Some(track_id),
                stated_decode_time: Some(stated),
                reached_decode_time: Some(reached),
                ..Fields::EMPTY
            },
            Self::SampleDescriptionIndexMismatch {
                track_id,
                stated,
                established,
            } => Fields {
                track_id: Some(track_id),
                sample_description_index: Some(stated),
                established_sample_description_index: Some(established),
                ..Fields::EMPTY
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::Category;

    use crate::error::Error;

    #[test]
    fn a_kind_falls_in_the_category_its_situation_asks_for() {
        assert_eq!(
            Error::decode_time_overflow(3).category(),
            Category::Malformed
        );
        assert_eq!(Error::unknown_track_id(3).category(), Category::Malformed);
        assert_eq!(
            Error::sample_size_limit_exceeded(1, 32, 16).category(),
            Category::Unsupported
        );
        assert_eq!(
            Error::external_data_reference(1, 2).category(),
            Category::Unsupported
        );
        assert_eq!(
            Error::first_chunk_out_of_range(1, 3).category(),
            Category::Malformed
        );
        assert_eq!(
            Error::sync_sample_out_of_range(1, 3).category(),
            Category::Malformed
        );
        assert_eq!(Error::already_finished().category(), Category::Usage);
        assert_eq!(
            Error::decode_time_mismatch(1, 512, 1_024).category(),
            Category::Malformed
        );
        assert_eq!(
            Error::sample_size_out_of_range(1, 1 << 40).category(),
            Category::Unsupported
        );
        assert_eq!(Error::no_fragment_open().category(), Category::Usage);
        assert_eq!(Error::no_chunk_open().category(), Category::Usage);
        assert_eq!(
            Error::track_id_mismatch(2, 1).category(),
            Category::Malformed
        );
        assert_eq!(
            Error::unsupported_sample_flags(1, 0x0200_0000).category(),
            Category::Unsupported
        );
        assert_eq!(
            Error::from(isobmff_core::Error::unsupported_version(2)).category(),
            Category::Unsupported
        );
    }
}
