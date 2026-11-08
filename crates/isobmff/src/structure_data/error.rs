//! Why a [`StructureDataReader`](super::StructureDataReader) refuses a box

use core::fmt;

use isobmff_core::BoxType;

/// Why a [`StructureDataReader`](super::StructureDataReader) refuses a box
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Error {
    /// The tracks declare more samples than
    /// [`StructureDataReader::new`](super::StructureDataReader::new) allows;
    /// nothing is allocated for them
    SampleCountOverLimit {
        /// Number of samples the tracks declare
        sample_count: u64,
        /// The most samples the reader resolves from one box
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
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SampleCountOverLimit {
                sample_count,
                limit,
            } => write!(
                formatter,
                "the tracks declare {sample_count} samples, over the limit of {limit}"
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
        }
    }
}

impl core::error::Error for Error {}
