//! Why a [`StructureDataReader`](super::StructureDataReader) refuses a box, or a [`SampleTableWriter`](super::SampleTableWriter) a sample

use core::fmt;

use isobmff_core::BoxType;

/// Why a [`StructureDataReader`](super::StructureDataReader) refuses a box, or a [`SampleTableWriter`](super::SampleTableWriter) a sample
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Error {
    /// The tracks declare more samples than
    /// [`StructureDataReader::new`](super::StructureDataReader::new) allows,
    /// and nothing is allocated for them; or a sample is one more than
    /// [`SampleTableWriter::new`](super::SampleTableWriter::new) allows, and
    /// it is not taken
    SampleCountOverLimit {
        /// Number of samples the tracks declare, or the writer would hold
        sample_count: u64,
        /// The most samples the reader resolves from one box, or the writer
        /// holds
        limit: u64,
    },
    /// A table the sample table of a track needs is missing
    MissingTable {
        /// `track_ID` of the track
        track_id: u32,
        /// Type of the box missing
        box_type: BoxType,
    },
    /// A table of a track counts another number of samples than the sample
    /// size table of the track does
    SampleCountMismatch {
        /// `track_ID` of the track
        track_id: u32,
        /// Type of the box whose count differs
        box_type: BoxType,
        /// Number of samples the sample size table counts
        sample_count: u64,
        /// Number of samples the box counts
        table_sample_count: u64,
    },
    /// An entry of a table of a track holds a value the table does not allow,
    /// such as a `first_chunk` that does not increase
    InvalidEntry {
        /// `track_ID` of the track
        track_id: u32,
        /// Type of the box
        box_type: BoxType,
        /// Index, from 0, of the entry
        entry: u64,
    },
    /// The samples of a track lie in another file than the one the box is in,
    /// or in an `IdentifiedMediaDataBox`
    MediaDataElsewhere {
        /// `track_ID` of the track
        track_id: u32,
        /// `data_reference_index`, from 1, of the sample entry of the samples
        data_reference_index: u16,
    },
    /// The bytes of a sample of a track lie past the last offset a file can
    /// have, or its decoding time past the last a track can have
    Overflow {
        /// `track_ID` of the track
        track_id: u32,
    },
    /// The `sample_composition_time_offset` of a sample does not fit the 32
    /// bits of a `ctts` entry, signed, ISO/IEC 14496-12 §8.6.1.3
    CompositionOffsetOutOfRange {
        /// `sample_composition_time_offset` of the sample
        sample_composition_time_offset: i64,
    },
    /// The `sample_description_index` of a sample names no entry of the
    /// `stsd` the sample table is written with
    SampleDescriptionIndexOutOfRange {
        /// `sample_description_index` of the sample, from 1
        sample_description_index: u32,
    },
    /// The sample table does not write
    Encode(isobmff_core::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SampleCountOverLimit {
                sample_count,
                limit,
            } => write!(
                formatter,
                "{sample_count} samples, over the limit of {limit}"
            ),
            Self::MissingTable { track_id, box_type } => write!(
                formatter,
                "the sample table of track {track_id} has no `{box_type}`"
            ),
            Self::SampleCountMismatch {
                track_id,
                box_type,
                sample_count,
                table_sample_count,
            } => write!(
                formatter,
                "the `{box_type}` of track {track_id} counts {table_sample_count} samples, and the sample size table {sample_count}"
            ),
            Self::InvalidEntry {
                track_id,
                box_type,
                entry,
            } => write!(
                formatter,
                "entry {entry} of the `{box_type}` of track {track_id} is not allowed"
            ),
            Self::MediaDataElsewhere {
                track_id,
                data_reference_index,
            } => write!(
                formatter,
                "the samples of track {track_id} lie where data reference {data_reference_index} names, not in this file"
            ),
            Self::Overflow { track_id } => write!(
                formatter,
                "a sample of track {track_id} lies past the last offset or decoding time there can be"
            ),
            Self::CompositionOffsetOutOfRange {
                sample_composition_time_offset,
            } => write!(
                formatter,
                "sample_composition_time_offset {sample_composition_time_offset} does not fit 32 bits, signed"
            ),
            Self::SampleDescriptionIndexOutOfRange {
                sample_description_index,
            } => write!(
                formatter,
                "sample_description_index {sample_description_index} names no entry of the `stsd`"
            ),
            Self::Encode(_) => write!(formatter, "the sample table does not write"),
        }
    }
}

impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Encode(source) => Some(source),
            Self::SampleCountOverLimit { .. }
            | Self::MissingTable { .. }
            | Self::SampleCountMismatch { .. }
            | Self::InvalidEntry { .. }
            | Self::MediaDataElsewhere { .. }
            | Self::Overflow { .. }
            | Self::CompositionOffsetOutOfRange { .. }
            | Self::SampleDescriptionIndexOutOfRange { .. } => None,
        }
    }
}
