//! Why the `structure-data` of a file is refused

use core::fmt;

/// Why the tables of the `structure-data` of a file are refused
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Error {
    /// A sync sample number of a track names no sample of the track: it is
    /// 0, or past the samples its sample size table declares, ISO/IEC
    /// 14496-12 §8.6.2.3
    SyncSampleNumberOutOfRange {
        /// `track_ID` of the track
        track_id: u32,
        /// Index, from 0, of the entry of the `stss`
        entry: u64,
        /// `sample_number` of the entry
        sample_number: u32,
        /// Number of the samples of the track
        sample_count: u32,
    },
    /// A sync sample number of a track is not after the one before it,
    /// though the sync sample table is in strictly increasing order of
    /// sample number, ISO/IEC 14496-12 §8.6.2.1
    SyncSampleNumberNotIncreasing {
        /// `track_ID` of the track
        track_id: u32,
        /// Index, from 0, of the entry of the `stss`
        entry: u64,
        /// `sample_number` of the entry
        sample_number: u32,
        /// `sample_number` of the entry before it
        previous_sample_number: u32,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SyncSampleNumberOutOfRange {
                track_id,
                entry,
                sample_number,
                sample_count,
            } => write!(
                formatter,
                "entry {entry} of the `stss` of track {track_id} has sample_number {sample_number}, which names none of its {sample_count} samples"
            ),
            Self::SyncSampleNumberNotIncreasing {
                track_id,
                entry,
                sample_number,
                previous_sample_number,
            } => write!(
                formatter,
                "entry {entry} of the `stss` of track {track_id} has sample_number {sample_number}, not after the sample_number {previous_sample_number} of the entry before it"
            ),
        }
    }
}

impl core::error::Error for Error {}
