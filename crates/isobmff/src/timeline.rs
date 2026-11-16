//! The timelines of a movie: the durations its headers declare, as its sample tables and edit lists give them, where its edits start in the media, and how they present its samples

use core::fmt;
use core::num::NonZeroU128;

use isobmff_boxes::{EditListBox, MovieBox};

use crate::structure_data::{composition_end, media_duration};

mod presentation;

pub use presentation::{Presentation, PresentedSample, presentation};

/// Makes the headers of `movie` declare the durations its sample tables and edit lists give, of the movie, of each of its tracks and of their media, each header in the version they need
///
/// * The duration of the media of a track is
///   [`media_duration`], ISO/IEC
///   14496-12 §8.4.2.
/// * The duration of a track is the sum of the durations of its edits, or,
///   without an edit list, the duration of its media in the timescale of the
///   movie, rounded up, §8.3.2.
/// * Where the last edit of a track is not empty and lasts 0, as the last
///   edit of a fragmented movie may to run to the end of its media
///   (§8.6.6.1), it is closed at the end of the composition of the samples:
///   it lasts from its `media_time` to the
///   [`composition_end`] of the
///   track, in the timescale of the movie, rounded up. The `elst` takes
///   version 1 where that duration does not fit 32 bits.
/// * The duration of the movie is that of its longest track, §8.2.2.
/// * The `mvhd`, each `tkhd` and each `mdhd` are of version 0 where their
///   duration, `creation_time` and `modification_time` fit 32 bits, and of
///   version 1 otherwise.
///
/// A media `timescale` of 0, which §8.4.2 does not allow, counts no time: a
/// track of such media without an edit list lasts 0, and its last edit of
/// duration 0 stays 0. Durations past what a `u64` counts are `u64::MAX`.
/// The `next_track_ID` and every other field are left as they are.
///
/// ```
/// use isobmff::boxes::{MovieBox, TimeToSampleBox, TimeToSampleEntry, TrackBox};
/// use isobmff::timeline::declare_durations;
///
/// // A track of 3 samples of 3,000 each, in media of 90,000 units a second
/// let mut run = TimeToSampleEntry::default();
/// (run.sample_count, run.sample_delta) = (3, 3_000);
/// let mut stts = TimeToSampleBox::default();
/// stts.entries = vec![run];
/// let mut trak = TrackBox::default();
/// trak.mdia.mdhd.timescale = 90_000;
/// trak.mdia.minf.stbl.stts = Some(stts);
/// // In a movie of 1,000 units a second
/// let mut moov = MovieBox::default();
/// moov.mvhd.timescale = 1_000;
/// moov.trak = vec![trak];
///
/// declare_durations(&mut moov);
/// assert_eq!(
///     (moov.mvhd.duration, moov.trak[0].tkhd.duration, moov.trak[0].mdia.mdhd.duration),
///     (100, 100, 9_000)
/// );
/// ```
pub fn declare_durations(movie: &mut MovieBox) {
    let movie_timescale = movie.mvhd.timescale;
    for trak in &mut movie.trak {
        let stbl = &trak.mdia.minf.stbl;
        let (media, composed) = (media_duration(stbl), composition_end(stbl));
        let media_timescale = trak.mdia.mdhd.timescale;
        let in_movie_timescale = |duration: u64| {
            // Why round up: the track then lasts at least as long as its
            // media, as FFmpeg's movenc rounds it.
            NonZeroU128::new(u128::from(media_timescale)).map_or(0, |timescale| {
                let scaled = u128::from(duration)
                    .saturating_mul(u128::from(movie_timescale))
                    .div_ceil(timescale.get());
                u64::try_from(scaled).unwrap_or(u64::MAX)
            })
        };
        if let Some(elst) = trak.edts.as_mut().and_then(|edts| edts.elst.as_mut()) {
            if let Some(last) = elst
                .entries
                .last_mut()
                .filter(|last| last.edit_duration == 0 && last.media_time >= 0)
            {
                last.edit_duration = in_movie_timescale(
                    composed.saturating_sub(u64::try_from(last.media_time).unwrap_or(0)),
                );
                if u32::try_from(last.edit_duration).is_err() {
                    elst.version = 1;
                }
            }
            trak.tkhd.duration = elst.entries.iter().fold(0_u64, |total, entry| {
                total.saturating_add(entry.edit_duration)
            });
        } else {
            trak.tkhd.duration = in_movie_timescale(media);
        }
        let tkhd = &mut trak.tkhd;
        tkhd.version = version_for([tkhd.duration, tkhd.creation_time, tkhd.modification_time]);
        let mdhd = &mut trak.mdia.mdhd;
        mdhd.duration = media;
        mdhd.version = version_for([mdhd.duration, mdhd.creation_time, mdhd.modification_time]);
    }
    let mvhd = &mut movie.mvhd;
    mvhd.duration = movie
        .trak
        .iter()
        .map(|trak| trak.tkhd.duration)
        .max()
        .unwrap_or(0);
    mvhd.version = version_for([mvhd.duration, mvhd.creation_time, mvhd.modification_time]);
}

/// The version of a box whose `values` take 32 bits in version 0 and 64 in version 1
fn version_for(values: [u64; 3]) -> u8 {
    u8::from(values.iter().any(|&value| u32::try_from(value).is_err()))
}

/// Moves the edits of `elst` that are not empty `earlier` time units back in the media, as they start where they did once the media starts `earlier` units sooner, and lasts until `media_end`
///
/// The `media_time` of an edit is the starting time within the media of
/// the edit, in composition time (ISO/IEC 14496-12 §8.6.6.3). Where the
/// samples of a track are declared to start `earlier` units sooner, as a
/// movie file starts the decoding times of a track at 0 (§8.6.1.2) where
/// its source did not, each edit that is not empty starts `earlier` units
/// sooner too. An empty edit, of `media_time` −1, is left as it is, and so
/// is the version of the `elst`.
///
/// # Errors
/// * [`Error::EditOutsideMedia`]: an edit that is not empty would start
///   before the media, or at or past `media_end`, where no sample is
///   composed: it points at media the file does not have. `elst` is left as
///   it was.
///
/// ```
/// use isobmff::boxes::{EditListBox, EditListEntry};
/// use isobmff::timeline::move_media_times;
///
/// // An empty edit, then an edit from 1,500 in media that starts at 1,000
/// let mut elst = EditListBox::default();
/// elst.entries = [(500, -1), (0, 1_500)]
///     .into_iter()
///     .map(|(edit_duration, media_time)| {
///         let mut entry = EditListEntry::default();
///         (entry.edit_duration, entry.media_time) = (edit_duration, media_time);
///         entry
///     })
///     .collect();
///
/// // The media now starts at 0 and lasts until 3,000
/// move_media_times(&mut elst, 1_000, 3_000)?;
/// assert_eq!(
///     elst.entries.iter().map(|entry| entry.media_time).collect::<Vec<_>>(),
///     [-1, 500]
/// );
/// # Ok::<(), isobmff::timeline::Error>(())
/// ```
pub fn move_media_times(elst: &mut EditListBox, earlier: u64, media_end: u64) -> Result<(), Error> {
    let moved = elst
        .entries
        .iter()
        .map(|entry| {
            if entry.media_time < 0 {
                return Ok(entry.media_time);
            }
            let moved = i128::from(entry.media_time).saturating_sub(i128::from(earlier));
            if moved < 0 || moved >= i128::from(media_end) {
                return Err(Error::EditOutsideMedia {
                    media_time: entry.media_time,
                    media_start: earlier,
                    media_end: earlier.saturating_add(media_end),
                });
            }
            // Why not refuse what does not fit: the value moved lies between
            // 0 and the `media_time` it was moved from, which an `i64` holds.
            Ok(i64::try_from(moved).unwrap_or(entry.media_time))
        })
        .collect::<Result<alloc::vec::Vec<_>, _>>()?;
    for (entry, media_time) in elst.entries.iter_mut().zip(moved) {
        entry.media_time = media_time;
    }
    Ok(())
}

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
