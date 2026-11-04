//! [`MuxError`], the reasons a muxer does not write samples into a file

use core::{error, fmt};

use isobmff::WriteError;
use isobmff_core::{BoxType, FourCc};

/// Reason a [`Muxer`](crate::Muxer) does not write samples into a file
///
/// Either the writers of the layers refuse the boxes ([`Write`](Self::Write)),
/// or the call breaks the order of the file the muxer keeps: the brands,
/// then the movie, then the samples. A refused call leaves the muxer as it
/// was: nothing of it is output.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MuxError {
    /// Boxes that do not write
    Write {
        /// Why they do not
        error: WriteError,
    },
    /// Box of a type the file holds one of, offered again
    DuplicateBox {
        /// Type of the box
        box_type: BoxType,
    },
    /// Box offered after a box it goes before in the file
    BoxOutOfOrder {
        /// Type of the box offered
        box_type: BoxType,
    },
    /// Samples, or the end of the file, offered before the movie that declares their tracks
    MissingMovie,
    /// Recoverable file that holds more samples than the muxer keeps the locations of, which it does not lay out again as a file that is not fragmented; the file written is a fragmented movie file whole
    NotFinalised {
        /// Number of samples written
        held_locations: u64,
        /// Number of samples the muxer keeps the locations of
        limit_locations: u64,
    },
    /// Brands of a fragmented file or a segment that name none of `iso5` to `isod`, under which a reader takes the `moof` as the base data offset of a track fragment, §8.8.7.1
    UnsupportedBrand {
        /// The major brand
        brand: FourCc,
    },
}

impl From<WriteError> for MuxError {
    fn from(error: WriteError) -> Self {
        Self::Write { error }
    }
}

impl fmt::Display for MuxError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Write { error } => fmt::Display::fmt(&error, formatter),
            Self::DuplicateBox { box_type } => write!(formatter, "{box_type} box offered again"),
            Self::BoxOutOfOrder { box_type } => write!(
                formatter,
                "{box_type} box offered after a box it goes before"
            ),
            Self::MissingMovie => formatter.write_str("samples offered before the movie"),
            Self::NotFinalised {
                held_locations,
                limit_locations,
            } => write!(
                formatter,
                "{held_locations} samples written, past the {limit_locations} whose locations the muxer keeps: the file stays fragmented"
            ),
            Self::UnsupportedBrand { brand } => write!(
                formatter,
                "brands of major brand {brand} name none of iso5 to isod, which a track fragment based on its moof takes"
            ),
        }
    }
}

impl error::Error for MuxError {}
