//! [`ErrorKind`], what a failure of the samples of a presentation is

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
pub enum ErrorKind {
    /// Failure of one box, carried through as `isobmff-core` names it
    ///
    /// The values that failure carries, and the boxes it was reached through,
    /// are on [`box_error`](crate::Error::box_error).
    Box(isobmff_core::ErrorKind),
    /// Decode times of a track run past what 64 bits carry
    ///
    /// [`track_id`](crate::Error::track_id) is the track they belong to.
    DecodeTimeOverflow,
    /// Data offsets of a track run past what 64 bits carry
    ///
    /// [`track_id`](crate::Error::track_id) is the track they belong to.
    DataOffsetOverflow,
    /// Fragment carries samples of a track the movie never declared
    ///
    /// A track is declared by a `trak` and, for its fragments, a `trex`
    /// (ISO/IEC 14496-12 §8.8.3); a fragment of a track missing either is
    /// refused. [`track_id`](crate::Error::track_id) is the track it names.
    UnknownTrackId,
    /// Samples are described by an `stsd` entry their track has none of
    ///
    /// A sample table names the entry by the run of chunks (ISO/IEC 14496-12
    /// §8.7.4), a fragment by its `tfhd` or the `trex` of the track (§8.8.7).
    /// [`track_id`](crate::Error::track_id) is the track they belong to, and
    /// [`sample_description_index`](crate::Error::sample_description_index) the
    /// entry named, counted from one.
    UnknownSampleDescriptionIndex,
    /// Movie carries no `mvex`, and so continues in no fragments
    ///
    /// A movie continued in fragments declares so by its `mvex` (ISO/IEC
    /// 14496-12 §8.8.1); a fragment of a movie carrying none is refused.
    MissingMovieExtends,
    /// Sample entry names a `dref` entry the track has none of
    ///
    /// [`track_id`](crate::Error::track_id) is the track it belongs to, and
    /// [`data_reference_index`](crate::Error::data_reference_index) the entry
    /// it names, counted from one.
    UnknownDataReferenceIndex,
    /// Data reference names a resource other than the file itself
    ///
    /// A `dref` entry flagged self-contained has the media data in the file
    /// that carries the movie (ISO/IEC 14496-12 §8.7.2); any other sends the
    /// reader to an external file, which no resolver here follows yet, so a
    /// sample described through one is refused.
    /// [`track_id`](crate::Error::track_id) is the track it belongs to, and
    /// [`data_reference_index`](crate::Error::data_reference_index) the entry,
    /// counted from one.
    ExternalDataReference,
    /// Sample tables of a track count different numbers of samples
    ///
    /// The `stts`, the `stsz`, and the `stsc` laid over the chunk offsets each count
    /// the samples of the track (ISO/IEC 14496-12 §8.6.1.2, §8.7.3.2, §8.7.4),
    /// and a track whose tables disagree is refused.
    /// [`track_id`](crate::Error::track_id) is the track.
    SampleCountMismatch,
    /// Run of chunks starts at a chunk outside the range open to it
    ///
    /// The first run an `stsc` states starts at chunk 1, and each run after it
    /// at a chunk past the start of the one before, no later than the last
    /// chunk the `stco` or the `co64` places (ISO/IEC 14496-12 §8.7.4.3).
    /// [`track_id`](crate::Error::track_id) is the track, and
    /// [`first_chunk`](crate::Error::first_chunk) the chunk the run states it
    /// starts at.
    FirstChunkOutOfRange,
    /// Sync sample is listed out of order, or past the samples of its track
    ///
    /// An `stss` lists the sync samples of a track in strictly increasing
    /// order of sample number (ISO/IEC 14496-12 §8.6.2), each a sample the
    /// track holds. [`track_id`](crate::Error::track_id) is the track, and
    /// [`sample_number`](crate::Error::sample_number) the sample number
    /// listed, counted from one.
    SyncSampleOutOfRange,
    /// Sample is declared past the limit the reader holds
    ///
    /// [`track_id`](crate::Error::track_id) is the track it belongs to,
    /// [`needed_bytes`](crate::Error::needed_bytes) the length it declares, and
    /// [`available_bytes`](crate::Error::available_bytes) the length the reader
    /// gathers for one sample at most.
    SampleSizeLimitExceeded,
    /// Samples were declared over while the bytes of one had still to arrive
    ///
    /// [`track_id`](crate::Error::track_id) is the track it belongs to,
    /// [`needed_bytes`](crate::Error::needed_bytes) the length it takes, and
    /// [`available_bytes`](crate::Error::available_bytes) the length that
    /// arrived.
    UnfinishedSample,
    /// Samples were declared over, and take nothing more
    AlreadyFinished,
    /// Sample was handed over, or a fragment closed, while no fragment was open
    NoFragmentOpen,
    /// Fragment was begun while the one before it was still open
    FragmentStillOpen,
    /// Sample is longer than the 32 bits a `trun` row or an `stsz` entry states its length in
    ///
    /// [`track_id`](crate::Error::track_id) is the track it belongs to,
    /// and [`needed_bytes`](crate::Error::needed_bytes) the length it
    /// carries.
    SampleSizeOutOfRange,
    /// Sample lies further into its fragment than the signed 32 bits of a `trun` offset reach
    ///
    /// [`MovieFragmentWriter`](crate::MovieFragmentWriter) anchors its offsets
    /// at the `moof` (ISO/IEC 14496-12 §8.8.7.1), so a fragment whose media
    /// data runs past what that field counts to is refused.
    /// [`track_id`](crate::Error::track_id) is the track it belongs to,
    /// and [`data_offset`](crate::Error::data_offset) how far into the
    /// fragment the sample lies.
    DataOffsetOutOfRange,
    /// Sample states a composition time offset no version of a `trun` or a `ctts` writes
    ///
    /// Version 0 of either box writes the offset unsigned in 32 bits and
    /// version 1 signed (ISO/IEC 14496-12 §8.8.8, §8.6.1.3), so one past both
    /// is refused. A `ctts` states the offsets of a whole track in one version,
    /// so a track stating a negative offset and one past [`i32::MAX`] is
    /// refused too, naming the widest.
    /// [`track_id`](crate::Error::track_id) is the track it belongs to,
    /// and [`composition_time_offset`](crate::Error::composition_time_offset)
    /// the offset it states.
    CompositionTimeOffsetOutOfRange,
    /// Sample does not start where the one before it in its track ends
    ///
    /// A `trun` states how long a sample lasts and not when it is decoded, so
    /// the decode times of the samples of one fragment are only written if each
    /// carries on from the one before it.
    /// [`track_id`](crate::Error::track_id) is the track it belongs to,
    /// [`stated_decode_time`](crate::Error::stated_decode_time) the decode
    /// time the sample states, and
    /// [`reached_decode_time`](crate::Error::reached_decode_time) the one
    /// the samples before it reach.
    DecodeTimeMismatch,
    /// Fragment of a track starts before the samples written for it reach
    ///
    /// The decode time a `tfdt` states never goes back: ISO/IEC 14496-12
    /// §8.8.12 has it as the sum of the durations of the samples before it,
    /// and a fragment starting past that sum is written as it stands, but one
    /// starting short of it is refused.
    /// [`track_id`](crate::Error::track_id) is the track it belongs to,
    /// [`stated_decode_time`](crate::Error::stated_decode_time) the decode
    /// time the fragment starts at, and
    /// [`reached_decode_time`](crate::Error::reached_decode_time) the one
    /// the samples written reach.
    BackwardDecodeTime,
    /// Samples of one fragment of one track, or of one chunk, are described by two `stsd` entries
    ///
    /// A `tfhd` states which entry describes the samples of its `traf` once,
    /// for all of them (ISO/IEC 14496-12 §8.8.7), and so does the run of chunks
    /// a chunk lies in (§8.7.4).
    /// [`track_id`](crate::Error::track_id) is the track they belong to,
    /// [`sample_description_index`](crate::Error::sample_description_index)
    /// the entry the sample that differed names, and
    /// [`established_sample_description_index`](crate::Error::established_sample_description_index)
    /// the one the fragment or the chunk describes the track by.
    SampleDescriptionIndexMismatch,
    /// Sample was handed over while no chunk was open
    NoChunkOpen,
    /// Sample belongs to another track than the chunk that is open holds
    ///
    /// A chunk is a contiguous set of samples of one track (ISO/IEC 14496-12
    /// §3.1.2), the one its first sample belongs to.
    /// [`track_id`](crate::Error::track_id) is the track the sample
    /// belongs to, and
    /// [`established_track_id`](crate::Error::established_track_id) the
    /// one the chunk holds.
    TrackIdMismatch,
    /// Sample sets a reserved bit of its flags, which no sample table carries
    ///
    /// A sample table states the fields of the `sample_flags` (ISO/IEC
    /// 14496-12 §8.8.3.1) in tables of their own, and none of them holds the
    /// 4 reserved bits, so a sample setting one is refused.
    /// [`track_id`](crate::Error::track_id) is the track it belongs to,
    /// and [`sample_flags`](crate::Error::sample_flags) the flags it
    /// states.
    UnsupportedSampleFlags,
}
