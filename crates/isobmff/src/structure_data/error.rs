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
    /// The first entry of a sample-to-chunk table has a `first_chunk` other
    /// than 1, ISO/IEC 14496-12 §8.7.4.3
    FirstChunkNotOne {
        /// `first_chunk` of the entry
        first_chunk: u32,
    },
    /// A `first_chunk` of a sample-to-chunk table names no chunk: it is 0, or
    /// past the chunks the chunk offset table gives, ISO/IEC 14496-12 §8.7.4.3
    FirstChunkOutOfRange {
        /// Index, from 0, of the entry of the `stsc`
        entry: u64,
        /// `first_chunk` of the entry
        first_chunk: u32,
        /// Number of the chunks
        chunk_count: u32,
    },
    /// A `first_chunk` of a sample-to-chunk table is not after the one before
    /// it, though each entry begins a run of chunks, ISO/IEC 14496-12 §8.7.4.1
    FirstChunkNotIncreasing {
        /// Index, from 0, of the entry of the `stsc`
        entry: u64,
        /// `first_chunk` of the entry
        first_chunk: u32,
        /// `first_chunk` of the entry before it
        previous_first_chunk: u32,
    },
    /// A `sample_description_index` names no sample entry: it is 0, or past
    /// the entries of the `stsd`, which an index counts from 1, ISO/IEC
    /// 14496-12 §7.3.1 and §8.7.4.3
    SampleDescriptionIndexOutOfRange {
        /// `sample_description_index` found
        sample_description_index: u32,
        /// Number of the sample entries of the `stsd`
        sample_entry_count: u32,
    },
    /// A sample entry ends before its `data_reference_index`, ISO/IEC
    /// 14496-12 §8.5.2.2
    DataReferenceIndexMissing {
        /// `sample_description_index`, from 1, of the sample entry
        sample_description_index: u32,
    },
    /// The `data_reference_index` of a sample entry names no data reference
    /// entry: it is 0, or past the entries of the `dref`, which ISO/IEC
    /// 14496-12 §8.5.2.3 numbers from 1
    DataReferenceIndexOutOfRange {
        /// `sample_description_index`, from 1, of the sample entry
        sample_description_index: u32,
        /// `data_reference_index` of the sample entry
        data_reference_index: u16,
        /// Number of the data reference entries of the `dref`
        data_entry_count: u32,
    },
    /// The sample entry of the samples of a sample table names a
    /// `DataEntrySeqNumImdaBox`, whose samples are each in a movie fragment,
    /// ISO/IEC 14496-12 §8.7.2.1
    SequenceNumberOutsideMovieFragment {
        /// `sample_description_index`, from 1, of the sample entry
        sample_description_index: u32,
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
            Self::FirstChunkNotOne { first_chunk } => write!(
                formatter,
                "entry 0 of the `stsc` has first_chunk {first_chunk}, not 1"
            ),
            Self::FirstChunkOutOfRange {
                entry,
                first_chunk,
                chunk_count,
            } => write!(
                formatter,
                "entry {entry} of the `stsc` has first_chunk {first_chunk}, which names none of the {chunk_count} chunks"
            ),
            Self::FirstChunkNotIncreasing {
                entry,
                first_chunk,
                previous_first_chunk,
            } => write!(
                formatter,
                "entry {entry} of the `stsc` has first_chunk {first_chunk}, not after the first_chunk {previous_first_chunk} of the entry before it"
            ),
            Self::SampleDescriptionIndexOutOfRange {
                sample_description_index,
                sample_entry_count,
            } => write!(
                formatter,
                "sample_description_index {sample_description_index} names none of the {sample_entry_count} sample entries"
            ),
            Self::DataReferenceIndexMissing {
                sample_description_index,
            } => write!(
                formatter,
                "sample entry {sample_description_index} ends before its data_reference_index"
            ),
            Self::DataReferenceIndexOutOfRange {
                sample_description_index,
                data_reference_index,
                data_entry_count,
            } => write!(
                formatter,
                "sample entry {sample_description_index} has data_reference_index {data_reference_index}, which names none of the {data_entry_count} data reference entries"
            ),
            Self::SequenceNumberOutsideMovieFragment {
                sample_description_index,
            } => write!(
                formatter,
                "sample entry {sample_description_index} of a sample table names a DataEntrySeqNumImdaBox, which only movie fragments use"
            ),
        }
    }
}

impl core::error::Error for Error {}
