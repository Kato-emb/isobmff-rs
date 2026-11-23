//! Why the `structure-data` of a file is refused

use core::fmt;

use isobmff_core::BoxType;

/// Why the tables of the `structure-data` of a file are refused
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Error {
    /// An entry of a table of a track holds a value the table does not allow,
    /// such as a sync sample number that names no sample of the track or is
    /// not after the one before it, ISO/IEC 14496-12 §8.6.2.3
    InvalidEntry {
        /// `track_ID` of the track
        track_id: u32,
        /// Type of the box
        box_type: BoxType,
        /// Index, from 0, of the entry
        entry: u64,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEntry {
                track_id,
                box_type,
                entry,
            } => write!(
                formatter,
                "entry {entry} of the `{box_type}` of track {track_id} is not allowed"
            ),
        }
    }
}

impl core::error::Error for Error {}
