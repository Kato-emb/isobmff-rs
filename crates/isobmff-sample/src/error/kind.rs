//! [`SampleErrorKind`], what a failure of the samples of a presentation is

use isobmff_core::Category;

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
    /// are on [`box_error`](crate::SampleError::box_error).
    Box(isobmff_core::ErrorKind),
    /// Decode times of a track run past what 64 bits carry
    ///
    /// [`track_id`](crate::SampleError::track_id) is the track they belong to.
    DecodeTimeOverflow,
    /// Data offsets of a track run past what 64 bits carry
    ///
    /// [`track_id`](crate::SampleError::track_id) is the track they belong to.
    DataOffsetOverflow,
    /// Fragment carries samples of a track the movie never declared
    ///
    /// A track is declared by a `trak` and, for its fragments, a `trex`
    /// (ISO/IEC 14496-12 §8.8.3); a fragment of a track missing either is
    /// refused. [`track_id`](crate::SampleError::track_id) is the track it names.
    UnknownTrackId,
    /// Samples are described by an `stsd` entry their track has none of
    ///
    /// A sample table names the entry by the run of chunks (ISO/IEC 14496-12
    /// §8.7.4), a fragment by its `tfhd` or the `trex` of the track (§8.8.7).
    /// [`track_id`](crate::SampleError::track_id) is the track they belong to, and
    /// [`sample_description_index`](crate::SampleError::sample_description_index) the
    /// entry named, counted from one.
    UnknownSampleDescriptionIndex,
    /// Movie carries no `mvex`, and so continues in no fragments
    ///
    /// A movie continued in fragments declares so by its `mvex` (ISO/IEC
    /// 14496-12 §8.8.1); a fragment of a movie carrying none is refused.
    MissingMovieExtends,
    /// Sample entry names a `dref` entry the track has none of
    ///
    /// [`track_id`](crate::SampleError::track_id) is the track it belongs to, and
    /// [`data_reference_index`](crate::SampleError::data_reference_index) the entry
    /// it names, counted from one.
    UnknownDataReferenceIndex,
    /// Data reference names a resource other than the file itself
    ///
    /// A `dref` entry flagged self-contained has the media data in the file
    /// that carries the movie (ISO/IEC 14496-12 §8.7.2); any other sends the
    /// reader to an external file, which no resolver here follows yet, so a
    /// sample described through one is refused.
    /// [`track_id`](crate::SampleError::track_id) is the track it belongs to, and
    /// [`data_reference_index`](crate::SampleError::data_reference_index) the entry,
    /// counted from one.
    ExternalDataReference,
    /// Sample tables of a track count different numbers of samples
    ///
    /// The `stts`, the `stsz`, and the `stsc` laid over the `stco` each count
    /// the samples of the track (ISO/IEC 14496-12 §8.6.1.2, §8.7.3.2, §8.7.4),
    /// and a track whose tables disagree is refused.
    /// [`track_id`](crate::SampleError::track_id) is the track.
    SampleCountMismatch,
    /// Run of chunks starts at a chunk outside the range open to it
    ///
    /// The first run an `stsc` states starts at chunk 1, and each run after it
    /// at a chunk past the start of the one before, no later than the last
    /// chunk the `stco` places (ISO/IEC 14496-12 §8.7.4.3).
    /// [`track_id`](crate::SampleError::track_id) is the track, and
    /// [`first_chunk`](crate::SampleError::first_chunk) the chunk the run states it
    /// starts at.
    FirstChunkOutOfRange,
    /// Sample is declared past the limit the reader holds
    ///
    /// [`track_id`](crate::SampleError::track_id) is the track it belongs to,
    /// [`needed_bytes`](crate::SampleError::needed_bytes) the length it declares, and
    /// [`available_bytes`](crate::SampleError::available_bytes) the length the reader
    /// gathers for one sample at most.
    SampleSizeLimitExceeded,
    /// Samples were declared over while the bytes of one had still to arrive
    ///
    /// [`track_id`](crate::SampleError::track_id) is the track it belongs to,
    /// [`needed_bytes`](crate::SampleError::needed_bytes) the length it takes, and
    /// [`available_bytes`](crate::SampleError::available_bytes) the length that
    /// arrived.
    UnfinishedSample,
    /// Samples were declared over, and take nothing more
    AlreadyFinished,
    /// Sample was handed over, or a fragment closed, while no fragment was open
    NoFragmentOpen,
    /// Fragment was begun while the one before it was still open
    FragmentStillOpen,
    /// Sample is longer than the 32 bits a `trun` row states its length in
    ///
    /// [`track_id`](crate::SampleError::track_id) is the track it belongs to,
    /// and [`needed_bytes`](crate::SampleError::needed_bytes) the length it
    /// carries.
    SampleSizeOutOfRange,
    /// Sample lies further into its fragment than the signed 32 bits of a `trun` offset reach
    ///
    /// The offsets of this writer are anchored at the `moof` (ISO/IEC 14496-12
    /// §8.8.7.1), so a fragment whose media data runs past what that field
    /// counts to is refused. [`track_id`](crate::SampleError::track_id) is the
    /// track it belongs to.
    DataOffsetOutOfRange,
    /// Sample states a composition time offset neither version of a `trun` writes
    ///
    /// Version 0 writes the offset unsigned in 32 bits and version 1 signed
    /// (ISO/IEC 14496-12 §8.8.8), so one past either is refused.
    /// [`track_id`](crate::SampleError::track_id) is the track it belongs to.
    CompositionTimeOffsetOutOfRange,
    /// Sample does not start where the one before it in its track ends
    ///
    /// A `trun` states how long a sample lasts and not when it is decoded, so
    /// the decode times of the samples of one fragment are only written if each
    /// carries on from the one before it.
    /// [`track_id`](crate::SampleError::track_id) is the track it belongs to.
    DecodeTimeMismatch,
    /// Fragment of a track starts before the samples written for it reach
    ///
    /// The decode time a `tfdt` states never goes back: ISO/IEC 14496-12
    /// §8.8.12 has it as the sum of the durations of the samples before it,
    /// and a fragment starting past that sum is written as it stands, but one
    /// starting short of it is refused.
    /// [`track_id`](crate::SampleError::track_id) is the track it belongs to.
    BackwardDecodeTime,
    /// Samples of one fragment of one track are described by two `stsd` entries
    ///
    /// A `traf` states which entry describes its samples once, for all of them.
    /// [`track_id`](crate::SampleError::track_id) is the track they belong to,
    /// and [`sample_description_index`](crate::SampleError::sample_description_index)
    /// the entry the sample that differed names.
    SampleDescriptionIndexMismatch,
    /// Samples do not build the boxes of a fragment
    ///
    /// The writer holds the samples it is given to what the boxes of a fragment
    /// state, so this is never reached from samples a caller hands over.
    FragmentNotRepresentable,
}

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
    /// Sample longer than the field a `trun` row states its length in reaches
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
    /// Samples of one fragment of one track described by two `stsd` entries
    SampleDescriptionIndexMismatch {
        track_id: u32,
        stated: u32,
        established: u32,
    },
    /// Samples that do not build the boxes of a fragment
    FragmentNotRepresentable,
}

impl Representation {
    /// Returns what went wrong
    pub(super) const fn kind(self) -> SampleErrorKind {
        match self {
            Self::Box(box_error) => SampleErrorKind::Box(box_error.kind()),
            Self::DecodeTimeOverflow { .. } => SampleErrorKind::DecodeTimeOverflow,
            Self::DataOffsetOverflow { .. } => SampleErrorKind::DataOffsetOverflow,
            Self::UnknownTrackId { .. } => SampleErrorKind::UnknownTrackId,
            Self::UnknownSampleDescriptionIndex { .. } => {
                SampleErrorKind::UnknownSampleDescriptionIndex
            }
            Self::MissingMovieExtends => SampleErrorKind::MissingMovieExtends,
            Self::UnknownDataReferenceIndex { .. } => SampleErrorKind::UnknownDataReferenceIndex,
            Self::ExternalDataReference { .. } => SampleErrorKind::ExternalDataReference,
            Self::SampleCountMismatch { .. } => SampleErrorKind::SampleCountMismatch,
            Self::FirstChunkOutOfRange { .. } => SampleErrorKind::FirstChunkOutOfRange,
            Self::SampleSizeLimitExceeded { .. } => SampleErrorKind::SampleSizeLimitExceeded,
            Self::UnfinishedSample { .. } => SampleErrorKind::UnfinishedSample,
            Self::AlreadyFinished => SampleErrorKind::AlreadyFinished,
            Self::NoFragmentOpen => SampleErrorKind::NoFragmentOpen,
            Self::FragmentStillOpen => SampleErrorKind::FragmentStillOpen,
            Self::SampleSizeOutOfRange { .. } => SampleErrorKind::SampleSizeOutOfRange,
            Self::DataOffsetOutOfRange { .. } => SampleErrorKind::DataOffsetOutOfRange,
            Self::CompositionTimeOffsetOutOfRange { .. } => {
                SampleErrorKind::CompositionTimeOffsetOutOfRange
            }
            Self::DecodeTimeMismatch { .. } => SampleErrorKind::DecodeTimeMismatch,
            Self::BackwardDecodeTime { .. } => SampleErrorKind::BackwardDecodeTime,
            Self::SampleDescriptionIndexMismatch { .. } => {
                SampleErrorKind::SampleDescriptionIndexMismatch
            }
            Self::FragmentNotRepresentable => SampleErrorKind::FragmentNotRepresentable,
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
            | Self::UnfinishedSample { .. }
            | Self::DecodeTimeMismatch { .. }
            | Self::BackwardDecodeTime { .. }
            | Self::SampleDescriptionIndexMismatch { .. } => Category::Malformed,
            Self::ExternalDataReference { .. }
            | Self::SampleSizeLimitExceeded { .. }
            | Self::SampleSizeOutOfRange { .. }
            | Self::DataOffsetOutOfRange { .. }
            | Self::CompositionTimeOffsetOutOfRange { .. } => Category::Unsupported,
            Self::AlreadyFinished
            | Self::NoFragmentOpen
            | Self::FragmentStillOpen
            | Self::FragmentNotRepresentable => Category::Usage,
        }
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::Category;

    use crate::error::SampleError;

    #[test]
    fn a_kind_falls_in_the_category_its_situation_asks_for() {
        assert_eq!(
            SampleError::decode_time_overflow(3).category(),
            Category::Malformed
        );
        assert_eq!(
            SampleError::unknown_track_id(3).category(),
            Category::Malformed
        );
        assert_eq!(
            SampleError::sample_size_limit_exceeded(1, 32, 16).category(),
            Category::Unsupported
        );
        assert_eq!(
            SampleError::external_data_reference(1, 2).category(),
            Category::Unsupported
        );
        assert_eq!(
            SampleError::first_chunk_out_of_range(1, 3).category(),
            Category::Malformed
        );
        assert_eq!(SampleError::already_finished().category(), Category::Usage);
        assert_eq!(
            SampleError::decode_time_mismatch(1, 512, 1_024).category(),
            Category::Malformed
        );
        assert_eq!(
            SampleError::sample_size_out_of_range(1, 1 << 40).category(),
            Category::Unsupported
        );
        assert_eq!(SampleError::no_fragment_open().category(), Category::Usage);
        assert_eq!(
            SampleError::from(isobmff_core::Error::unsupported_version(2)).category(),
            Category::Unsupported
        );
    }
}
