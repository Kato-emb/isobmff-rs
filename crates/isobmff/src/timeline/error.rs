//! [`Error`], why the edits of a track cannot be moved or cannot present its samples

use core::fmt;

/// Why the edits of a track cannot be moved where its media now starts, or cannot present its samples
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Error {
    /// An edit that is not empty starts outside the media: before its first
    /// sample, or where the composition of its samples has ended, ISO/IEC
    /// 14496-12 §8.6.6
    EditOutsideMedia {
        /// `media_time` of the edit, where the media started before it moved
        media_time: i64,
        /// Where the media started before it moved
        media_start: u64,
        /// Where the composition of the samples ends, where the media
        /// started before it moved
        media_end: u64,
    },
    /// The movie has no track of this `track_ID`
    UnknownTrack {
        /// `track_ID` asked for
        track_id: u32,
    },
    /// A sample is of another track than the one asked for
    OtherTrack {
        /// `track_ID` of the track asked for
        track_id: u32,
        /// `track_ID` of the sample
        found: u32,
    },
    /// The track has an edit list, and the movie or its media has a
    /// `timescale` of 0, which ISO/IEC 14496-12 §8.2.2 and §8.4.2 do not
    /// allow: its edits cannot be written in the timescale of the media
    ZeroTimescale {
        /// `timescale` of the `mvhd`
        movie_timescale: u32,
        /// `timescale` of the `mdhd` of the track
        media_timescale: u32,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EditOutsideMedia {
                media_time,
                media_start,
                media_end,
            } => write!(
                formatter,
                "an edit starts at media_time {media_time}, outside the media, which runs from {media_start} to {media_end}"
            ),
            Self::UnknownTrack { track_id } => {
                write!(formatter, "the movie has no track {track_id}")
            }
            Self::OtherTrack { track_id, found } => write!(
                formatter,
                "a sample of track {found} is among those of track {track_id}"
            ),
            Self::ZeroTimescale {
                movie_timescale,
                media_timescale,
            } => write!(
                formatter,
                "the movie timescale is {movie_timescale} and the media timescale {media_timescale}, and an edit list needs both above 0"
            ),
        }
    }
}

impl core::error::Error for Error {}
