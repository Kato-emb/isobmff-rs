//! [`Error`], what a [`Demuxer`](super::Demuxer) cannot read

use core::fmt;

use isobmff_core::BoxType;

use crate::{file_level, structure_data};

/// What a [`Demuxer`](super::Demuxer) cannot read
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Error {
    /// A box at file level cannot be read; reading goes on where the error
    /// says
    FileLevel(file_level::Error),
    /// A box the demuxer reads into a value does not read; reading goes on at
    /// the box after it
    Decode {
        /// File offset of the box
        offset: u64,
        /// Type of the box
        box_type: BoxType,
        /// Why the box does not read
        source: isobmff_core::Error,
    },
    /// The samples a `moov` declares cannot be located as the specification
    /// reads them; neither the box nor the location of any of its samples is
    /// reported
    StructureData {
        /// File offset of the `moov`
        offset: u64,
        /// Why its samples cannot be located
        source: structure_data::Error,
    },
    /// A `moov` follows the one already read, which a file holds one of,
    /// ISO/IEC 14496-12 §8.2.1; it is not read, and the first is kept
    ///
    /// A second `moov` that does not read is reported as
    /// [`Decode`](Self::Decode) instead.
    DuplicateMovie {
        /// File offset of the `moov` that follows
        offset: u64,
    },
    /// A `moov` declares more samples than
    /// [`DemuxerConfig::sample_count_limit`](super::DemuxerConfig::sample_count_limit)
    /// lets the demuxer hold the locations of; neither the box nor the
    /// location of any of its samples is reported, and nothing is allocated
    /// for them
    SampleCountOverLimit {
        /// File offset of the `moov`
        offset: u64,
        /// The sample locations the demuxer would hold once it read the box
        location_count: u64,
        /// The limit they are over
        limit: u64,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileLevel(_) => write!(formatter, "a box at file level cannot be read"),
            Self::Decode {
                offset, box_type, ..
            } => write!(
                formatter,
                "the box `{box_type}` at offset {offset} does not read"
            ),
            Self::StructureData { offset, .. } => write!(
                formatter,
                "the samples of the `moov` at offset {offset} cannot be located as the specification reads them"
            ),
            Self::DuplicateMovie { offset } => write!(
                formatter,
                "the `moov` at offset {offset} follows the one already read"
            ),
            Self::SampleCountOverLimit {
                offset,
                location_count,
                limit,
            } => write!(
                formatter,
                "the `moov` at offset {offset} would have the demuxer hold {location_count} sample locations, over the limit of {limit}"
            ),
        }
    }
}

impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::FileLevel(source) => Some(source),
            Self::Decode { source, .. } => Some(source),
            Self::StructureData { source, .. } => Some(source),
            Self::DuplicateMovie { .. } | Self::SampleCountOverLimit { .. } => None,
        }
    }
}
