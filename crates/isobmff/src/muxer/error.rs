//! Why a muxer refuses a value

use core::fmt;

use crate::structure_data;

/// Why a muxer refuses a value; a refused call changes nothing
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Error {
    /// The muxer has finished, and takes no more
    Finished,
    /// A track is added after a sample was taken
    TrackAfterMediaData {
        /// `track_ID` of the track
        track_id: u32,
    },
    /// A track is added with the `track_ID` of 0, which ISO/IEC 14496-12
    /// §8.3.2 does not allow, or of a track already added
    TrackId {
        /// `track_ID` of the track
        track_id: u32,
    },
    /// A sample entry of a track names another data reference than the one
    /// the muxer writes, the media data in the same file
    DataReferenceIndex {
        /// `track_ID` of the track
        track_id: u32,
        /// `data_reference_index` of the sample entry
        data_reference_index: u16,
    },
    /// A sample is of a track not added
    UnknownTrack {
        /// `track_ID` of the sample
        track_id: u32,
    },
    /// The sample table of the track refuses the sample, as where it names a
    /// sample entry the track does not have
    StructureData(structure_data::Error),
    /// A sample is longer than a sample table counts, or the media data would
    /// reach past the last offset a file can have
    Overflow,
    /// A sample would grow the bytes held for `poll_output` past
    /// [`MuxerConfig::buffer_limit`](super::MuxerConfig::buffer_limit)
    BufferFull {
        /// The bytes the output held would grow to
        length: usize,
    },
    /// A box does not write
    Encode(isobmff_core::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Finished => write!(formatter, "the muxer has finished"),
            Self::TrackAfterMediaData { track_id } => write!(
                formatter,
                "track {track_id} is added after a sample was taken"
            ),
            Self::TrackId { track_id } => write!(
                formatter,
                "track_ID {track_id} is 0 or that of a track already added"
            ),
            Self::DataReferenceIndex {
                track_id,
                data_reference_index,
            } => write!(
                formatter,
                "a sample entry of track {track_id} has data_reference_index {data_reference_index}, not 1"
            ),
            Self::UnknownTrack { track_id } => {
                write!(formatter, "a sample is of track {track_id}, not added")
            }
            Self::StructureData(_) => {
                write!(formatter, "the sample table refuses the sample")
            }
            Self::Overflow => write!(
                formatter,
                "a sample is longer than a sample table counts, or ends past the last offset a file can have"
            ),
            Self::BufferFull { length } => write!(
                formatter,
                "the output held would grow to {length} bytes, past buffer_limit"
            ),
            Self::Encode(_) => write!(formatter, "a box does not write"),
        }
    }
}

impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::StructureData(source) => Some(source),
            Self::Encode(source) => Some(source),
            Self::Finished
            | Self::TrackAfterMediaData { .. }
            | Self::TrackId { .. }
            | Self::DataReferenceIndex { .. }
            | Self::UnknownTrack { .. }
            | Self::Overflow
            | Self::BufferFull { .. } => None,
        }
    }
}
