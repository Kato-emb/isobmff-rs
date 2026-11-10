//! The timelines of a movie: how long its media, its tracks and the movie last, as their sample tables and edit lists give it

use core::num::NonZeroU128;

use isobmff_boxes::MovieBox;

use crate::structure_data::{composition_end, media_duration};

/// Sets the durations of `movie`, of each of its tracks and of their media to those the sample tables and the edit lists give, and each header to the version they need
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
/// use isobmff::timeline::set_durations;
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
/// set_durations(&mut moov);
/// assert_eq!(
///     (moov.mvhd.duration, moov.trak[0].tkhd.duration, moov.trak[0].mdia.mdhd.duration),
///     (100, 100, 9_000)
/// );
/// ```
pub fn set_durations(movie: &mut MovieBox) {
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
