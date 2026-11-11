//! [`Mismatch`], where the boxes a reader took disagree with themselves, and which side the reader took

use isobmff_core::BoxType;

/// Where the boxes a reader took disagree with themselves, as FFmpeg and GPAC read them
///
/// A reader that takes a box of such a disagreement resolves its samples as
/// the variant says, and goes on. It refuses only a sample table under which
/// no sample can be placed.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mismatch {
    /// A table of the sample table of a track counts another number of
    /// samples than the sample size table does, ISO/IEC 14496-12 §8.7.3
    ///
    /// The samples are those both the sample size table and the chunks
    /// place, §8.7.4, as many as the fewer of the two counts. Past the end
    /// of the time-to-sample table, §8.6.1.2, each sample lasts as long as
    /// its last; past the end of the composition offset table, §8.6.1.3,
    /// a sample has an offset of 0; past the end of the sample dependency
    /// table, §8.6.4, its dependencies are unknown. What a table declares
    /// past the samples is not read.
    SampleCount {
        /// `track_ID` of the track
        track_id: u32,
        /// Type of the table whose count differs
        box_type: BoxType,
        /// Number of samples the sample size table counts
        sample_count: u64,
        /// Number of samples the table counts
        table_sample_count: u64,
    },
    /// The `tfdt` of a track fragment gives another decoding time than the
    /// durations of the samples before it in the track add up to, §8.8.12
    ///
    /// The samples of the track fragment take the time the `tfdt` gives; the
    /// samples before them are left as they were returned.
    DecodingTime {
        /// `track_ID` of the track
        track_id: u32,
        /// Decoding time the durations of the samples before add up to
        expected: u64,
        /// Decoding time the `tfdt` gives
        found: u64,
    },
    /// An entry of a table of a track names no sample of the track, or does
    /// not follow the entry before it, and is not read
    ///
    /// Such as a number in the sync sample table, §8.6.2, past the last
    /// sample, or no greater than the one before it.
    Entry {
        /// `track_ID` of the track
        track_id: u32,
        /// Type of the table
        box_type: BoxType,
        /// Index, from 0, of the entry
        entry: u64,
    },
}
