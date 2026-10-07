//! [`Error`], the reason the samples of a presentation do not resolve

use core::error;
use core::fmt;

use isobmff_core::Category;

/// Reason the samples of a presentation do not resolve
///
/// What went wrong is one variant, which carries the values that describe it:
/// a failure of the samples themselves — a fragment of a track the movie never
/// declared, a timeline or an offset run past what its field carries, a sample
/// that never arrived whole — or a failure of one box, which
/// [`isobmff_core::Error`] names and this type carries through whole, as
/// [`Box`](Self::Box). What a caller does about either is one
/// [`category`](Self::category).
///
/// The vocabulary is this crate's own: resolving where a sample lies,
/// gathering it, and laying it out in a declaration name their failures here.
/// The situations this layer reaches are added to as ISO/IEC 14496-12 is read
/// further, so a match on this must leave room for variants that are not here
/// yet, and a match on a variant must leave room for fields that are not here
/// yet.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::SampleFlags;
/// use isobmff_core::{BoxType, Category};
/// use isobmff_sample::{Error, SampleExtent, SampleReader};
///
/// // A sample whose bytes never arrived whole is a failure of the samples themselves
/// let mut reader = SampleReader::new();
/// reader.handle_sample_extent(SampleExtent::new(3, 0, 1_024, 0, SampleFlags::ZERO, 1, 1, 100..104))?;
/// reader.handle_data(100, b"AB")?;
/// let failure = reader.finish().unwrap_err();
/// assert!(matches!(
///     failure,
///     Error::UnfinishedSample {
///         track_id: 3,
///         needed_bytes: 4,
///         available_bytes: 2,
///         ..
///     }
/// ));
/// assert_eq!(failure.category(), Category::Malformed);
///
/// // A failure of one box is carried through whole
/// let missing = isobmff_core::Error::missing_mandatory_box(BoxType::compact(*b"trex"));
/// let carried = Error::from(missing);
/// assert!(matches!(carried, Error::Box { error, .. } if error == missing));
/// assert_eq!(carried.category(), Category::Malformed);
/// # Ok::<(), Error>(())
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Error {
    /// Failure of one box, carried through as `isobmff-core` names it
    #[non_exhaustive]
    Box {
        /// Failure of the box, carrying its own values and the boxes it was reached through
        error: isobmff_core::Error,
    },
    /// Decode times of a track run past what 64 bits carry
    #[non_exhaustive]
    DecodeTimeOverflow {
        /// Track the decode times belong to
        track_id: u32,
    },
    /// Presentation times of a track run past what 64 bits carry
    ///
    /// A `sidx` states the presentation time of its first subsegment and the
    /// duration of each (ISO/IEC 14496-12 §8.16.3), which sum to when each
    /// subsegment starts.
    #[non_exhaustive]
    PresentationTimeOverflow {
        /// Stream the index names by its `reference_ID`
        track_id: u32,
    },
    /// Track fragment states no decode time, and where its track stands is not known
    ///
    /// A `traf` carrying no `tfdt` carries on from the durations of every
    /// sample of its track before it (ISO/IEC 14496-12 §8.8.12), which a
    /// reader that did not start at the first fragment has not summed; see
    /// [`TrackDecodeTimes::unknown`](crate::TrackDecodeTimes::unknown).
    #[non_exhaustive]
    MissingDecodeTime {
        /// Track the fragment belongs to
        track_id: u32,
    },
    /// Data offsets of a track run past what 64 bits carry
    ///
    /// For the subsegments of a `sidx` (ISO/IEC 14496-12 §8.16.3), the track
    /// is the stream the index names by its `reference_ID`.
    #[non_exhaustive]
    DataOffsetOverflow {
        /// Track the data offsets belong to
        track_id: u32,
    },
    /// Sample belongs to a track the movie never declared
    ///
    /// A track is declared by a `trak` and, for its fragments, a `trex`
    /// (ISO/IEC 14496-12 §8.8.3); a fragment of a track missing either is
    /// refused, as is a sample handed to a writer whose track the movie
    /// declares no `trak` for, or, for fragments, no `trex`. Where the movie
    /// keeps a `trak` unread, a fragment of a track no read `trak` declares
    /// gives no sample instead, and this names such a track only where a
    /// fragment after it needs where its data ends and nothing states the
    /// sizes of its samples.
    #[non_exhaustive]
    UnknownTrackId {
        /// Track named
        track_id: u32,
    },
    /// Samples are described by an `stsd` entry their track has none of
    ///
    /// A sample table names the entry by the run of chunks (ISO/IEC 14496-12
    /// §8.7.4), a fragment by its `tfhd` or the `trex` of the track (§8.8.7).
    #[non_exhaustive]
    UnknownSampleDescriptionIndex {
        /// Track the samples belong to
        track_id: u32,
        /// Entry named, counted from one
        sample_description_index: u32,
    },
    /// Movie carries no `mvex`, and so continues in no fragments
    ///
    /// A movie continued in fragments declares so by its `mvex` (ISO/IEC
    /// 14496-12 §8.8.1); a fragment of a movie carrying none is refused.
    #[non_exhaustive]
    MissingMovieExtends,
    /// Movie continued in fragments lays samples out in the sample table of a track
    ///
    /// The `stts`, `stsc`, sample sizes or chunk offsets of a track of a movie
    /// continued in fragments (ISO/IEC 14496-12 §8.8.1) lay a sample out,
    /// which a writer laying out fragments refuses.
    #[non_exhaustive]
    SampleTableNotEmpty {
        /// Track whose sample table lays samples out
        track_id: u32,
    },
    /// Sample entry names a `dref` entry the track has none of
    #[non_exhaustive]
    UnknownDataReferenceIndex {
        /// Track the sample entry belongs to
        track_id: u32,
        /// Entry named, counted from one
        data_reference_index: u16,
    },
    /// Data reference names a resource other than the file itself
    ///
    /// A `dref` entry flagged self-contained has the media data in the file
    /// that carries the movie (ISO/IEC 14496-12 §8.7.2); any other sends the
    /// reader to an external file, which no resolver here follows yet, so a
    /// sample described through one is refused. An entry held as
    /// [`DataEntry::Other`](isobmff_boxes::DataEntry::Other) does not read as
    /// the file itself, so it counts as such an entry.
    #[non_exhaustive]
    ExternalDataReference {
        /// Track the data reference belongs to
        track_id: u32,
        /// Entry naming the resource, counted from one
        data_reference_index: u16,
    },
    /// Sample tables of a track count different numbers of samples
    ///
    /// The `stts`, the `stsz` or the `stz2`, the `stsc` laid over the chunk
    /// offsets, and any `ctts`, `sdtp`, `padb` or `stdp` each count the samples
    /// of the track (ISO/IEC 14496-12 §8.6.1, §8.7.3, §8.7.4, §8.6.4, §8.7.6,
    /// §8.5.3), and a track whose tables disagree is refused.
    #[non_exhaustive]
    SampleCountMismatch {
        /// Track whose tables disagree
        track_id: u32,
    },
    /// Run of chunks starts at a chunk outside the range open to it
    ///
    /// The first run an `stsc` states starts at chunk 1, and each run after it
    /// at a chunk past the start of the one before, no later than the last
    /// chunk the `stco` or the `co64` places (ISO/IEC 14496-12 §8.7.4.3).
    #[non_exhaustive]
    FirstChunkOutOfRange {
        /// Track the run belongs to
        track_id: u32,
        /// Chunk the run states it starts at
        first_chunk: u32,
    },
    /// Sync sample is listed out of order, or past the samples of its track
    ///
    /// An `stss` lists the sync samples of a track in strictly increasing
    /// order of sample number (ISO/IEC 14496-12 §8.6.2), each a sample the
    /// track holds.
    #[non_exhaustive]
    SyncSampleOutOfRange {
        /// Track the sync samples belong to
        track_id: u32,
        /// Sample number listed, counted from one
        sample_number: u32,
    },
    /// Sample is declared past the limit the reader holds
    #[non_exhaustive]
    SampleSizeLimitExceeded {
        /// Track the sample belongs to
        track_id: u32,
        /// Length the sample declares
        declared_bytes: u64,
        /// Length the reader gathers for one sample at most
        limit_bytes: u64,
    },
    /// Boxes declare more samples than one resolution lays out
    ///
    /// The count is that of the call, every track of a movie or every run of
    /// a fragment together, compared before a sample is laid out, so it names
    /// no track.
    #[non_exhaustive]
    SampleCountLimitExceeded {
        /// Count of samples the boxes declare
        declared_samples: u64,
        /// Most samples the caller lets one call lay out
        limit_samples: u64,
    },
    /// Extent would be held past the count the reader holds
    #[non_exhaustive]
    HeldExtentLimitExceeded {
        /// Count of extents the reader would hold with it
        needed_extents: u64,
        /// Most extents the reader holds
        limit_extents: u64,
    },
    /// Sample would be gathered past the bytes the reader holds
    ///
    /// The bytes held are those of every sample the reader has begun
    /// gathering and not yet handed over, whole or not.
    #[non_exhaustive]
    HeldBytesLimitExceeded {
        /// Length the reader would hold with the sample
        needed_bytes: u64,
        /// Most bytes the reader holds
        limit_bytes: u64,
    },
    /// Samples were declared over while the bytes of one had still to arrive
    #[non_exhaustive]
    UnfinishedSample {
        /// Track the sample belongs to
        track_id: u32,
        /// Length the sample takes
        needed_bytes: u64,
        /// Length that arrived
        available_bytes: u64,
    },
    /// Samples were declared over, and take nothing more
    #[non_exhaustive]
    AlreadyFinished,
    /// Sample was handed over, or a fragment closed, while no fragment was open
    #[non_exhaustive]
    NoFragmentOpen,
    /// Fragment was begun while the one before it was still open
    #[non_exhaustive]
    FragmentStillOpen,
    /// Sample is longer than the 32 bits a `trun` row or an `stsz` entry states its length in
    #[non_exhaustive]
    SampleSizeOutOfRange {
        /// Track the sample belongs to
        track_id: u32,
        /// Length the sample carries
        declared_bytes: u64,
    },
    /// Sample lies further into its fragment than the signed 32 bits of a `trun` offset reach
    ///
    /// [`MovieFragmentWriter`](crate::MovieFragmentWriter) anchors its offsets
    /// at the `moof` (ISO/IEC 14496-12 §8.8.7.1), so a fragment whose media
    /// data runs past what that field counts to is refused.
    #[non_exhaustive]
    DataOffsetOutOfRange {
        /// Track the sample belongs to
        track_id: u32,
        /// How far into the fragment the sample lies, in bytes
        data_offset: u64,
    },
    /// Sample states a composition time offset outside what 32 signed bits hold
    ///
    /// Version 0 of a `trun` or a `ctts` writes the offset unsigned in 32 bits
    /// and version 1 signed (ISO/IEC 14496-12 §8.8.8, §8.6.1.3). The writers
    /// refuse an offset below [`i32::MIN`], and one past [`i32::MAX`], which the
    /// readers here take as the negative value of the same bits.
    #[non_exhaustive]
    CompositionTimeOffsetOutOfRange {
        /// Track the sample belongs to
        track_id: u32,
        /// Offset the sample states
        composition_time_offset: i64,
    },
    /// Sample does not start where the one before it in its track ends
    ///
    /// A `trun` states how long a sample lasts and not when it is decoded, so
    /// the decode times of the samples of one fragment are only written if each
    /// carries on from the one before it. In a fragment opened by
    /// [`begin_fragment_continuing`](crate::MovieFragmentWriter::begin_fragment_continuing)
    /// the reached decode time is counted, like the stated one, from the decode
    /// time the first sample of the track in the fragment states, not from
    /// where the fragment places it.
    #[non_exhaustive]
    DecodeTimeMismatch {
        /// Track the sample belongs to
        track_id: u32,
        /// Decode time the sample states
        stated_decode_time: u64,
        /// Decode time the samples before it reach
        reached_decode_time: u64,
    },
    /// Fragment of a track starts before the samples written for it reach
    ///
    /// The decode time a `tfdt` states never goes back: ISO/IEC 14496-12
    /// §8.8.12 has it as the sum of the durations of the samples before it,
    /// and a fragment starting past that sum is written as it stands, but one
    /// starting short of it is refused.
    #[non_exhaustive]
    BackwardDecodeTime {
        /// Track the fragment belongs to
        track_id: u32,
        /// Decode time the fragment starts at
        stated_decode_time: u64,
        /// Decode time the samples written reach
        reached_decode_time: u64,
    },
    /// Samples of one fragment of one track, or of one chunk, are described by two `stsd` entries
    ///
    /// A `tfhd` states which entry describes the samples of its `traf` once,
    /// for all of them (ISO/IEC 14496-12 §8.8.7), and so does the run of chunks
    /// a chunk lies in (§8.7.4).
    #[non_exhaustive]
    SampleDescriptionIndexMismatch {
        /// Track the samples belong to
        track_id: u32,
        /// Entry the sample that differed names
        stated_sample_description_index: u32,
        /// Entry the fragment or the chunk describes the track by
        established_sample_description_index: u32,
    },
    /// Sample was handed over while no chunk was open
    #[non_exhaustive]
    NoChunkOpen,
    /// Sample belongs to another track than the chunk that is open holds
    ///
    /// A chunk is a contiguous set of samples of one track (ISO/IEC 14496-12
    /// §3.1.2), the one its first sample belongs to.
    #[non_exhaustive]
    TrackIdMismatch {
        /// Track the sample belongs to
        stated_track_id: u32,
        /// Track the chunk holds
        established_track_id: u32,
    },
}

impl Error {
    /// Returns what a caller does about the failure
    #[must_use]
    pub const fn category(self) -> Category {
        match self {
            Self::Box { error } => error.category(),
            Self::DecodeTimeOverflow { .. }
            | Self::PresentationTimeOverflow { .. }
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
            | Self::SampleTableNotEmpty { .. }
            | Self::MissingDecodeTime { .. }
            | Self::SampleSizeLimitExceeded { .. }
            | Self::SampleCountLimitExceeded { .. }
            | Self::HeldExtentLimitExceeded { .. }
            | Self::HeldBytesLimitExceeded { .. }
            | Self::SampleSizeOutOfRange { .. }
            | Self::DataOffsetOutOfRange { .. }
            | Self::CompositionTimeOffsetOutOfRange { .. } => Category::Unsupported,
            Self::AlreadyFinished
            | Self::NoFragmentOpen
            | Self::FragmentStillOpen
            | Self::NoChunkOpen => Category::Usage,
        }
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
            Self::Box { error } => write!(formatter, "{error}"),
            Self::DecodeTimeOverflow { track_id } => write!(
                formatter,
                "decode time of track {track_id} runs past what 64 bits carry"
            ),
            Self::PresentationTimeOverflow { track_id } => write!(
                formatter,
                "presentation time of track {track_id} runs past what 64 bits carry"
            ),
            Self::MissingDecodeTime { track_id } => write!(
                formatter,
                "fragment of track {track_id} states no decode time, and where the track stands is not known"
            ),
            Self::DataOffsetOverflow { track_id } => write!(
                formatter,
                "data offset of track {track_id} runs past what 64 bits carry"
            ),
            Self::UnknownTrackId { track_id } => {
                write!(formatter, "movie declares no track {track_id}")
            }
            Self::UnknownSampleDescriptionIndex {
                track_id,
                sample_description_index,
            } => write!(
                formatter,
                "track {track_id} has no stsd entry {sample_description_index}"
            ),
            Self::MissingMovieExtends => formatter.write_str("the movie carries no mvex"),
            Self::SampleTableNotEmpty { track_id } => write!(
                formatter,
                "sample table of track {track_id} lays samples out in a movie continued in fragments"
            ),
            Self::UnknownDataReferenceIndex {
                track_id,
                data_reference_index,
            } => write!(
                formatter,
                "track {track_id} has no dref entry {data_reference_index}"
            ),
            Self::ExternalDataReference {
                track_id,
                data_reference_index,
            } => write!(
                formatter,
                "dref entry {data_reference_index} of track {track_id} names an external file"
            ),
            Self::SampleCountMismatch { track_id } => write!(
                formatter,
                "sample tables of track {track_id} count different numbers of samples"
            ),
            Self::FirstChunkOutOfRange {
                track_id,
                first_chunk,
            } => write!(
                formatter,
                "run of chunks of track {track_id} starts at chunk {first_chunk}, out of the range open to it"
            ),
            Self::SyncSampleOutOfRange {
                track_id,
                sample_number,
            } => write!(
                formatter,
                "sync sample {sample_number} of track {track_id} is listed out of order or past its samples"
            ),
            Self::SampleSizeLimitExceeded {
                track_id,
                declared_bytes,
                limit_bytes,
            } => write!(
                formatter,
                "track {track_id} declares a sample of {declared_bytes} bytes, past the {limit_bytes}-byte limit"
            ),
            Self::SampleCountLimitExceeded {
                declared_samples,
                limit_samples,
            } => write!(
                formatter,
                "boxes declare {declared_samples} samples, past the {limit_samples}-sample limit"
            ),
            Self::HeldExtentLimitExceeded {
                needed_extents,
                limit_extents,
            } => write!(
                formatter,
                "reader would hold {needed_extents} extents, past the {limit_extents}-extent limit"
            ),
            Self::HeldBytesLimitExceeded {
                needed_bytes,
                limit_bytes,
            } => write!(
                formatter,
                "reader would hold {needed_bytes} bytes, past the {limit_bytes}-byte limit"
            ),
            Self::UnfinishedSample {
                track_id,
                needed_bytes,
                available_bytes,
            } => write!(
                formatter,
                "sample of track {track_id} takes {needed_bytes} bytes, and {available_bytes} arrived"
            ),
            Self::AlreadyFinished => {
                formatter.write_str("samples were declared over and take nothing more")
            }
            Self::NoFragmentOpen => {
                formatter.write_str("no fragment is open to carry a sample or be closed")
            }
            Self::FragmentStillOpen => formatter.write_str("fragment is still open"),
            Self::SampleSizeOutOfRange {
                track_id,
                declared_bytes,
            } => write!(
                formatter,
                "track {track_id} states a sample of {declared_bytes} bytes, past the {} a trun row or an stsz entry carries",
                u32::MAX
            ),
            Self::DataOffsetOutOfRange {
                track_id,
                data_offset,
            } => write!(
                formatter,
                "sample data of track {track_id} lies {data_offset} bytes into the fragment, past the {} a trun carries",
                i32::MAX
            ),
            Self::CompositionTimeOffsetOutOfRange {
                track_id,
                composition_time_offset,
            } => write!(
                formatter,
                "track {track_id} states a composition time offset of {composition_time_offset}, outside what 32 signed bits hold"
            ),
            Self::DecodeTimeMismatch {
                track_id,
                stated_decode_time,
                reached_decode_time,
            } => write!(
                formatter,
                "track {track_id} states decode time {stated_decode_time} where the samples before it reach {reached_decode_time}"
            ),
            Self::BackwardDecodeTime {
                track_id,
                stated_decode_time,
                reached_decode_time,
            } => write!(
                formatter,
                "track {track_id} goes back to decode time {stated_decode_time} after reaching {reached_decode_time}"
            ),
            Self::SampleDescriptionIndexMismatch {
                track_id,
                stated_sample_description_index,
                established_sample_description_index,
            } => write!(
                formatter,
                "track {track_id} describes a sample by stsd entry {stated_sample_description_index} in a fragment or a chunk describing it by {established_sample_description_index}"
            ),
            Self::NoChunkOpen => formatter.write_str("no chunk is open to carry a sample"),
            Self::TrackIdMismatch {
                stated_track_id,
                established_track_id,
            } => write!(
                formatter,
                "sample of track {stated_track_id} handed over to a chunk of track {established_track_id}"
            ),
        }
    }
}

impl error::Error for Error {
    /// Returns the failure of one box carried through, when it holds one
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Box { error } => Some(error),
            Self::DecodeTimeOverflow { .. }
            | Self::PresentationTimeOverflow { .. }
            | Self::MissingDecodeTime { .. }
            | Self::DataOffsetOverflow { .. }
            | Self::UnknownTrackId { .. }
            | Self::UnknownSampleDescriptionIndex { .. }
            | Self::MissingMovieExtends
            | Self::SampleTableNotEmpty { .. }
            | Self::UnknownDataReferenceIndex { .. }
            | Self::ExternalDataReference { .. }
            | Self::SampleCountMismatch { .. }
            | Self::FirstChunkOutOfRange { .. }
            | Self::SyncSampleOutOfRange { .. }
            | Self::SampleSizeLimitExceeded { .. }
            | Self::SampleCountLimitExceeded { .. }
            | Self::HeldExtentLimitExceeded { .. }
            | Self::HeldBytesLimitExceeded { .. }
            | Self::UnfinishedSample { .. }
            | Self::AlreadyFinished
            | Self::NoFragmentOpen
            | Self::FragmentStillOpen
            | Self::SampleSizeOutOfRange { .. }
            | Self::DataOffsetOutOfRange { .. }
            | Self::CompositionTimeOffsetOutOfRange { .. }
            | Self::DecodeTimeMismatch { .. }
            | Self::BackwardDecodeTime { .. }
            | Self::SampleDescriptionIndexMismatch { .. }
            | Self::NoChunkOpen
            | Self::TrackIdMismatch { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString as _;

    use isobmff_core::{BoxType, Category};

    use crate::error::Error;

    #[test]
    fn a_failure_falls_in_the_category_its_situation_asks_for() {
        assert_eq!(
            Error::DecodeTimeOverflow { track_id: 3 }.category(),
            Category::Malformed
        );
        assert_eq!(
            Error::UnknownTrackId { track_id: 3 }.category(),
            Category::Malformed
        );
        assert_eq!(
            Error::PresentationTimeOverflow { track_id: 3 }.category(),
            Category::Malformed
        );
        assert_eq!(
            Error::MissingDecodeTime { track_id: 3 }.category(),
            Category::Unsupported
        );
        assert_eq!(
            Error::SampleSizeLimitExceeded {
                track_id: 1,
                declared_bytes: 32,
                limit_bytes: 16,
            }
            .category(),
            Category::Unsupported
        );
        assert_eq!(
            Error::SampleCountLimitExceeded {
                declared_samples: 32,
                limit_samples: 16,
            }
            .category(),
            Category::Unsupported
        );
        assert_eq!(
            Error::HeldExtentLimitExceeded {
                needed_extents: 17,
                limit_extents: 16,
            }
            .category(),
            Category::Unsupported
        );
        assert_eq!(
            Error::HeldBytesLimitExceeded {
                needed_bytes: 32,
                limit_bytes: 16,
            }
            .category(),
            Category::Unsupported
        );
        assert_eq!(
            Error::SampleTableNotEmpty { track_id: 4 }.category(),
            Category::Unsupported
        );
        assert_eq!(
            Error::ExternalDataReference {
                track_id: 1,
                data_reference_index: 2,
            }
            .category(),
            Category::Unsupported
        );
        assert_eq!(
            Error::FirstChunkOutOfRange {
                track_id: 1,
                first_chunk: 3,
            }
            .category(),
            Category::Malformed
        );
        assert_eq!(
            Error::SyncSampleOutOfRange {
                track_id: 1,
                sample_number: 3,
            }
            .category(),
            Category::Malformed
        );
        assert_eq!(Error::AlreadyFinished.category(), Category::Usage);
        assert_eq!(
            Error::DecodeTimeMismatch {
                track_id: 1,
                stated_decode_time: 512,
                reached_decode_time: 1_024,
            }
            .category(),
            Category::Malformed
        );
        assert_eq!(
            Error::SampleSizeOutOfRange {
                track_id: 1,
                declared_bytes: 1 << 40,
            }
            .category(),
            Category::Unsupported
        );
        assert_eq!(Error::NoFragmentOpen.category(), Category::Usage);
        assert_eq!(Error::NoChunkOpen.category(), Category::Usage);
        assert_eq!(
            Error::TrackIdMismatch {
                stated_track_id: 2,
                established_track_id: 1,
            }
            .category(),
            Category::Malformed
        );
        assert_eq!(
            Error::from(isobmff_core::Error::unsupported_version(2)).category(),
            Category::Unsupported
        );
    }

    #[test]
    fn display_of_a_failure_of_the_samples_states_the_reason() {
        assert_eq!(
            Error::DecodeTimeOverflow { track_id: 1 }.to_string(),
            "decode time of track 1 runs past what 64 bits carry"
        );
        assert_eq!(
            Error::PresentationTimeOverflow { track_id: 1 }.to_string(),
            "presentation time of track 1 runs past what 64 bits carry"
        );
        assert_eq!(
            Error::MissingDecodeTime { track_id: 1 }.to_string(),
            "fragment of track 1 states no decode time, and where the track stands is not known"
        );
        assert_eq!(
            Error::DataOffsetOverflow { track_id: 1 }.to_string(),
            "data offset of track 1 runs past what 64 bits carry"
        );
        assert_eq!(
            Error::UnknownTrackId { track_id: 3 }.to_string(),
            "movie declares no track 3"
        );
        assert_eq!(
            Error::UnknownSampleDescriptionIndex {
                track_id: 2,
                sample_description_index: 7,
            }
            .to_string(),
            "track 2 has no stsd entry 7"
        );
        assert_eq!(
            Error::MissingMovieExtends.to_string(),
            "the movie carries no mvex"
        );
        assert_eq!(
            Error::SampleTableNotEmpty { track_id: 4 }.to_string(),
            "sample table of track 4 lays samples out in a movie continued in fragments"
        );
        assert_eq!(
            Error::UnknownDataReferenceIndex {
                track_id: 2,
                data_reference_index: 3,
            }
            .to_string(),
            "track 2 has no dref entry 3"
        );
        assert_eq!(
            Error::ExternalDataReference {
                track_id: 2,
                data_reference_index: 3,
            }
            .to_string(),
            "dref entry 3 of track 2 names an external file"
        );
        assert_eq!(
            Error::SampleCountMismatch { track_id: 2 }.to_string(),
            "sample tables of track 2 count different numbers of samples"
        );
        assert_eq!(
            Error::FirstChunkOutOfRange {
                track_id: 2,
                first_chunk: 5,
            }
            .to_string(),
            "run of chunks of track 2 starts at chunk 5, out of the range open to it"
        );
        assert_eq!(
            Error::SyncSampleOutOfRange {
                track_id: 2,
                sample_number: 5,
            }
            .to_string(),
            "sync sample 5 of track 2 is listed out of order or past its samples"
        );
        assert_eq!(
            Error::SampleSizeLimitExceeded {
                track_id: 1,
                declared_bytes: 32,
                limit_bytes: 16,
            }
            .to_string(),
            "track 1 declares a sample of 32 bytes, past the 16-byte limit"
        );
        assert_eq!(
            Error::SampleCountLimitExceeded {
                declared_samples: 32,
                limit_samples: 16,
            }
            .to_string(),
            "boxes declare 32 samples, past the 16-sample limit"
        );
        assert_eq!(
            Error::HeldExtentLimitExceeded {
                needed_extents: 17,
                limit_extents: 16,
            }
            .to_string(),
            "reader would hold 17 extents, past the 16-extent limit"
        );
        assert_eq!(
            Error::HeldBytesLimitExceeded {
                needed_bytes: 32,
                limit_bytes: 16,
            }
            .to_string(),
            "reader would hold 32 bytes, past the 16-byte limit"
        );
        assert_eq!(
            Error::UnfinishedSample {
                track_id: 2,
                needed_bytes: 1_024,
                available_bytes: 512,
            }
            .to_string(),
            "sample of track 2 takes 1024 bytes, and 512 arrived"
        );
        assert_eq!(
            Error::AlreadyFinished.to_string(),
            "samples were declared over and take nothing more"
        );
        assert_eq!(
            Error::NoFragmentOpen.to_string(),
            "no fragment is open to carry a sample or be closed"
        );
        assert_eq!(
            Error::FragmentStillOpen.to_string(),
            "fragment is still open"
        );
        assert_eq!(
            Error::SampleSizeOutOfRange {
                track_id: 1,
                declared_bytes: 1 << 40,
            }
            .to_string(),
            "track 1 states a sample of 1099511627776 bytes, past the 4294967295 a trun row or an stsz entry carries"
        );
        assert_eq!(
            Error::DataOffsetOutOfRange {
                track_id: 1,
                data_offset: 1 << 40,
            }
            .to_string(),
            "sample data of track 1 lies 1099511627776 bytes into the fragment, past the 2147483647 a trun carries"
        );
        assert_eq!(
            Error::CompositionTimeOffsetOutOfRange {
                track_id: 1,
                composition_time_offset: 1 << 40,
            }
            .to_string(),
            "track 1 states a composition time offset of 1099511627776, outside what 32 signed bits hold"
        );
        assert_eq!(
            Error::DecodeTimeMismatch {
                track_id: 1,
                stated_decode_time: 512,
                reached_decode_time: 1_024,
            }
            .to_string(),
            "track 1 states decode time 512 where the samples before it reach 1024"
        );
        assert_eq!(
            Error::BackwardDecodeTime {
                track_id: 1,
                stated_decode_time: 512,
                reached_decode_time: 1_024,
            }
            .to_string(),
            "track 1 goes back to decode time 512 after reaching 1024"
        );
        assert_eq!(
            Error::SampleDescriptionIndexMismatch {
                track_id: 1,
                stated_sample_description_index: 2,
                established_sample_description_index: 1,
            }
            .to_string(),
            "track 1 describes a sample by stsd entry 2 in a fragment or a chunk describing it by 1"
        );
        assert_eq!(
            Error::NoChunkOpen.to_string(),
            "no chunk is open to carry a sample"
        );
        assert_eq!(
            Error::TrackIdMismatch {
                stated_track_id: 2,
                established_track_id: 1,
            }
            .to_string(),
            "sample of track 2 handed over to a chunk of track 1"
        );
    }

    #[test]
    fn display_of_a_failure_of_one_box_reads_as_that_failure() {
        let box_error = isobmff_core::Error::missing_mandatory_box(BoxType::compact(*b"mvex"));

        assert_eq!(Error::from(box_error).to_string(), box_error.to_string());
    }
}
