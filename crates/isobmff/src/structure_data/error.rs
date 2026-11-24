//! Why the `structure-data` of a file is refused

use alloc::boxed::Box;
use core::fmt;

use isobmff_core::BoxType;

/// Why the tables of the `structure-data` of a file are refused
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Error {
    /// The `structure-data` of a track is refused, for `source`, ISO/IEC 14496-12 §3.1.54
    Track {
        /// `track_ID` of the track
        track_id: u32,
        /// Why the tables are refused, which is not itself a `Track`
        source: Box<Error>,
    },
    /// A box a sample table holds, ISO/IEC 14496-12 §8.5, is absent, or is of
    /// a variant this crate does not read
    ///
    /// The time-to-sample table, the sample-to-chunk table, a chunk offset
    /// table and a sample size table are each mandatory, §8.6.1.2, §8.7.4,
    /// §8.7.5 and §8.7.3. A chunk offset table is named `stco` whether it is
    /// one of 32 or of 64 bits (`co64`), and a sample size table `stsz`
    /// whether it is compact (`stz2`) or not.
    MandatoryBoxMissing {
        /// Type of the box
        box_type: BoxType,
    },
    /// A table of a sample table covers another number of samples than the
    /// sample size table, whose `sample_count` gives the number of samples
    /// in the track, ISO/IEC 14496-12 §8.7.3.2
    ///
    /// A time-to-sample or composition offset table covers the
    /// `sample_count`s of its entries, §8.6.1.2 and §8.6.1.3, and the
    /// sample dependency types one sample for each entry, §8.6.4. The
    /// sample-to-chunk table declares no number of samples: it covers those
    /// its runs of chunks hold, `samples_per_chunk` for each chunk, §8.7.4.
    SampleCountMismatch {
        /// Type of the table: `stts`, `ctts`, `sdtp` or `stsc`
        box_type: BoxType,
        /// Number of the samples the table covers
        table_sample_count: u64,
        /// `sample_count` of the sample size table
        sample_count: u32,
    },
    /// The end of the bytes of a sample, the data offset just past the last of them, is past what a `u64` counts
    DataOffsetOverflow {
        /// `sample number`, from 1, of the sample, ISO/IEC 14496-12 §3.1.49
        sample_number: u32,
        /// Data offset of the first byte of the sample
        offset: u64,
        /// Number of the bytes of the sample
        size: u32,
    },
    /// A sync sample number names no sample: it is 0, or past the samples
    /// the sample size table declares, ISO/IEC 14496-12 §8.6.2.3
    SyncSampleNumberOutOfRange {
        /// Index, from 0, of the entry of the `stss`
        entry: u64,
        /// `sample_number` of the entry
        sample_number: u32,
        /// Number of the samples of the track
        sample_count: u32,
    },
    /// A sync sample number is not after the one before it, though the sync
    /// sample table is in strictly increasing order of sample number, ISO/IEC
    /// 14496-12 §8.6.2.1
    SyncSampleNumberNotIncreasing {
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
    /// The sample entry of a sample in no movie fragment names a
    /// `DataEntrySeqNumImdaBox`, whose samples are each in a movie fragment,
    /// ISO/IEC 14496-12 §8.7.2.1
    SequenceNumberOutsideMovieFragment {
        /// `sample_description_index`, from 1, of the sample entry
        sample_description_index: u32,
    },
    /// The data reference entry a sample entry names is a `DataEntryImdaBox`
    /// or a `DataEntrySeqNumImdaBox` that does not decode, whose type places
    /// the media data of its samples in an `IdentifiedMediaDataBox`, ISO/IEC
    /// 14496-12 §8.7.2.1
    DataEntryNotDecoded {
        /// `sample_description_index`, from 1, of the sample entry
        sample_description_index: u32,
        /// Type of the data reference entry
        box_type: BoxType,
    },
    /// A sample counts its data offset from a movie fragment, and no movie
    /// fragment was taken, ISO/IEC 14496-12 §8.8.7.1 Table 6
    MovieFragmentPositionMissing,
    /// A sample lies in an `IdentifiedMediaDataBox` that has not been read,
    /// ISO/IEC 14496-12 §8.1.4 and §8.7.2.1
    IdentifiedMediaDataNotRead {
        /// `imda_identifier` of the `IdentifiedMediaDataBox`
        imda_identifier: u32,
    },
    /// The end of the bytes of a sample, the file offset just past the last of them, is past what a `u64` counts
    ///
    /// The file offset of the data origin of the sample, added to its data
    /// offset and its size, gives the end.
    FileOffsetOverflow {
        /// File offset of the data origin
        data_origin_offset: u64,
        /// Data offset of the first byte of the sample, from the data origin
        offset: u64,
        /// Number of the bytes of the sample
        size: u32,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Track { track_id, .. } => {
                write!(
                    formatter,
                    "the structure-data of track {track_id} is refused"
                )
            }
            Self::MandatoryBoxMissing { box_type } => write!(
                formatter,
                "the sample table holds no `{box_type}` this crate reads, which is mandatory"
            ),
            Self::SampleCountMismatch {
                box_type,
                table_sample_count,
                sample_count,
            } => write!(
                formatter,
                "the `{box_type}` covers {table_sample_count} samples, not the sample_count {sample_count} of the sample size table"
            ),
            Self::DataOffsetOverflow {
                sample_number,
                offset,
                size,
            } => write!(
                formatter,
                "sample {sample_number} at data offset {offset} has size {size}, so the data offset just past its last byte is past what a u64 counts"
            ),
            Self::SyncSampleNumberOutOfRange {
                entry,
                sample_number,
                sample_count,
            } => write!(
                formatter,
                "entry {entry} of the `stss` has sample_number {sample_number}, which names none of the {sample_count} samples"
            ),
            Self::SyncSampleNumberNotIncreasing {
                entry,
                sample_number,
                previous_sample_number,
            } => write!(
                formatter,
                "entry {entry} of the `stss` has sample_number {sample_number}, not after the sample_number {previous_sample_number} of the entry before it"
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
            Self::DataEntryNotDecoded {
                sample_description_index,
                box_type,
            } => write!(
                formatter,
                "sample entry {sample_description_index} names a data reference entry `{box_type}` that does not decode"
            ),
            Self::MovieFragmentPositionMissing => write!(
                formatter,
                "a sample counts its data offset from a movie fragment, and no movie fragment was taken"
            ),
            Self::IdentifiedMediaDataNotRead { imda_identifier } => write!(
                formatter,
                "a sample lies in the IdentifiedMediaDataBox of imda_identifier {imda_identifier}, which has not been read"
            ),
            Self::FileOffsetOverflow {
                data_origin_offset,
                offset,
                size,
            } => write!(
                formatter,
                "a sample at data offset {offset} from a data origin at file offset {data_origin_offset} has size {size}, so the file offset just past its last byte is past what a u64 counts"
            ),
            Self::SequenceNumberOutsideMovieFragment {
                sample_description_index,
            } => write!(
                formatter,
                "sample entry {sample_description_index} names a DataEntrySeqNumImdaBox for a sample in no movie fragment"
            ),
        }
    }
}

impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Track { source, .. } => Some(source.as_ref()),
            Self::MandatoryBoxMissing { .. }
            | Self::SampleCountMismatch { .. }
            | Self::DataOffsetOverflow { .. }
            | Self::SyncSampleNumberOutOfRange { .. }
            | Self::SyncSampleNumberNotIncreasing { .. }
            | Self::FirstChunkNotOne { .. }
            | Self::FirstChunkOutOfRange { .. }
            | Self::FirstChunkNotIncreasing { .. }
            | Self::SampleDescriptionIndexOutOfRange { .. }
            | Self::DataReferenceIndexMissing { .. }
            | Self::DataReferenceIndexOutOfRange { .. }
            | Self::SequenceNumberOutsideMovieFragment { .. }
            | Self::DataEntryNotDecoded { .. }
            | Self::MovieFragmentPositionMissing
            | Self::IdentifiedMediaDataNotRead { .. }
            | Self::FileOffsetOverflow { .. } => None,
        }
    }
}
