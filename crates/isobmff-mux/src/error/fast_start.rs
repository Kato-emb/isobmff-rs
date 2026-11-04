//! [`FastStartError`], the reasons a movie file does not lay out again with its `moov` first

use core::{error, fmt};

use isobmff::{ReadError, WriteError};

/// Reason a movie file does not lay out again with its `moov` first
///
/// A refused input leaves the machine failed: it wants no more input, and
/// returns no output.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FastStartError {
    /// Boxes of the file that do not read
    Read {
        /// Why they do not
        error: ReadError,
    },
    /// `moov` laid out again that does not write, as a [`Muxer`](crate::Muxer) writes one and fails to
    Write {
        /// Why it does not
        error: WriteError,
    },
    /// File that has ended without a `moov`
    MissingMovie,
    /// Fragmented movie file: a `moof` at file level, or a `moov` with an `mvex`, §8.8
    FragmentedMovie,
    /// `ItemLocationBox` that locates items by file offsets in the file itself, which the machine does not move, §8.11.3
    ItemLocationsInFile,
    /// Chunk or sample auxiliary information offset in the file itself that lies in no box of the file, §8.7.5, §8.7.9
    OffsetOutOfFile {
        /// `track_ID` of the track
        track_id: u32,
        /// File offset the track states
        offset: u64,
    },
    /// Chunk or sample auxiliary information offset into the `moov` itself, which laying the `moov` out again does not keep in place, §8.7.5, §8.7.9
    OffsetInMovie {
        /// `track_ID` of the track
        track_id: u32,
        /// File offset the track states
        offset: u64,
    },
    /// Box at file level past the number the machine keeps the places of
    BoxCountLimitExceeded {
        /// Number of boxes at file level the machine keeps the places of
        limit_boxes: u64,
    },
}

impl fmt::Display for FastStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Read { error } => fmt::Display::fmt(&error, formatter),
            Self::Write { error } => fmt::Display::fmt(&error, formatter),
            Self::MissingMovie => formatter.write_str("a file without a movie"),
            Self::FragmentedMovie => formatter.write_str("a fragmented movie file"),
            Self::ItemLocationsInFile => {
                formatter.write_str("items located by file offsets in the file itself")
            }
            Self::OffsetOutOfFile { track_id, offset } => write!(
                formatter,
                "file offset {offset} of track {track_id}, in no box of the file"
            ),
            Self::OffsetInMovie { track_id, offset } => write!(
                formatter,
                "file offset {offset} of track {track_id}, into the movie laid out again"
            ),
            Self::BoxCountLimitExceeded { limit_boxes } => {
                write!(formatter, "more than {limit_boxes} boxes at file level")
            }
        }
    }
}

impl error::Error for FastStartError {}
