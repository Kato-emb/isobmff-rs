//! [`Error`], what a [`Demuxer`](super::Demuxer) cannot read

use core::fmt;

use isobmff_core::BoxType;

use crate::{SampleLocation, framing, structure_data};

/// What a [`Demuxer`](super::Demuxer) cannot read
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Error {
    /// A box at file level cannot be framed
    Framing(framing::Error),
    /// A box the demuxer reads into a value does not read
    Decode {
        /// File offset of the box
        offset: u64,
        /// Type of the box
        box_type: BoxType,
        /// Why the box does not read
        source: isobmff_core::Error,
    },
    /// The samples a `moov` or a `moof` declares cannot be resolved as the
    /// specification reads them; the box and the location of each of its
    /// samples follow where they resolve when read lossily, and none
    /// otherwise. Or an `imda` cannot be taken for the samples that lie in
    /// it, which are then not resolved from it
    StructureData {
        /// File offset of the box
        offset: u64,
        /// Type of the box, `moov`, `moof` or `imda`
        box_type: BoxType,
        /// Why its samples cannot be resolved
        source: structure_data::Error,
    },
    /// A `moov` follows the one already read, which a file holds one of,
    /// ISO/IEC 14496-12 §8.2.1; it is not read, and the first is kept
    DuplicateMovie {
        /// File offset of the `moov` that follows
        offset: u64,
    },
    /// A sample of a subscribed track is larger than
    /// [`DemuxerConfig::sample_size_limit`](super::DemuxerConfig::sample_size_limit);
    /// its bytes are not requested, and demuxing goes on at the next sample
    SampleOverLimit {
        /// Where the sample lies
        location: SampleLocation,
        /// The limit the sample is larger than
        limit: u64,
    },
    /// The input ends inside a sample of a subscribed track
    TruncatedSample {
        /// Where the sample lies
        location: SampleLocation,
    },
    /// The `tfra`s of an `mfra` declare more entries than
    /// [`DemuxerConfig::random_access_entry_limit`](super::DemuxerConfig::random_access_entry_limit);
    /// the box is not read, and nothing is allocated for its entries
    RandomAccessEntryOverLimit {
        /// File offset of the `mfra`
        offset: u64,
        /// The entries its `tfra`s declare
        entry_count: u64,
        /// The limit they are over
        limit: u64,
    },
    /// What the structure-data refuses once the input ends, such as samples
    /// that lie in an `imda` the file does not hold; they are not resolved
    EndOfInput(structure_data::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Framing(_) => write!(formatter, "a box at file level cannot be framed"),
            Self::EndOfInput(_) => write!(
                formatter,
                "the structure-data cannot be resolved at the end of the input"
            ),
            Self::Decode {
                offset, box_type, ..
            } => write!(
                formatter,
                "the box `{box_type}` at offset {offset} does not read"
            ),
            Self::StructureData {
                offset, box_type, ..
            } => write!(
                formatter,
                "the samples of the `{box_type}` at offset {offset} cannot be resolved as the specification reads them"
            ),
            Self::DuplicateMovie { offset } => write!(
                formatter,
                "the `moov` at offset {offset} follows the one already read"
            ),
            Self::SampleOverLimit { location, limit } => write!(
                formatter,
                "the sample of track {} at offset {} is {} bytes, over the limit of {limit}",
                location.properties.track_id, location.offset, location.size
            ),
            Self::TruncatedSample { location } => write!(
                formatter,
                "the input ends inside the sample of track {} at offset {}",
                location.properties.track_id, location.offset
            ),
            Self::RandomAccessEntryOverLimit {
                offset,
                entry_count,
                limit,
            } => write!(
                formatter,
                "the `mfra` at offset {offset} declares {entry_count} entries, over the limit of {limit}"
            ),
        }
    }
}

impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Framing(source) => Some(source),
            Self::Decode { source, .. } => Some(source),
            Self::StructureData { source, .. } => Some(source),
            Self::EndOfInput(source) => Some(source),
            Self::DuplicateMovie { .. }
            | Self::SampleOverLimit { .. }
            | Self::TruncatedSample { .. }
            | Self::RandomAccessEntryOverLimit { .. } => None,
        }
    }
}
