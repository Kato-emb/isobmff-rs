//! [`TimelineError`], the reasons the edit list of a track does not map its composition timeline to its presentation timeline

use core::error;
use core::fmt;

/// Reason the edit list of a track does not map its composition timeline to its presentation timeline, ISO/IEC 14496-12 §8.6.6
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TimelineError {
    /// Track the movie has none of
    #[non_exhaustive]
    UnknownTrackId {
        /// `track_ID` asked for
        track_id: u32,
    },
    /// Edit whose `media_time` is negative and not the −1 of an empty edit
    #[non_exhaustive]
    InvalidMediaTime {
        /// `track_ID` of the track
        track_id: u32,
        /// `media_time` of the edit
        media_time: i64,
    },
    /// Edit list of a movie whose `mvhd` states a timescale of 0, in which its edit durations count
    #[non_exhaustive]
    MissingMovieTimescale {
        /// `track_ID` of the track
        track_id: u32,
    },
}

impl fmt::Display for TimelineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::UnknownTrackId { track_id } => {
                write!(formatter, "no track {track_id} in the movie")
            }
            Self::InvalidMediaTime {
                track_id,
                media_time,
            } => write!(
                formatter,
                "edit of track {track_id} at media time {media_time}, which is neither an empty edit nor a time in the media"
            ),
            Self::MissingMovieTimescale { track_id } => write!(
                formatter,
                "edit list of track {track_id} in a movie of timescale 0, in which its edit durations count"
            ),
        }
    }
}

impl error::Error for TimelineError {}
