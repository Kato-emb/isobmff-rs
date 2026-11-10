//! The durations a `moov` declares, set from its sample tables and edit lists, and the movie extends box of its tracks

use core::num::NonZeroU128;

use isobmff_boxes::{MovieBox, MovieExtendsBox, SampleTableBox, TrackBox, TrackExtendsBox};

/// Sets the durations of `movie`, of each of its tracks and of their media to those the sample tables and the edit lists give, and each header to the version they need
///
/// * The duration of the media of a track is the sum of the durations of
///   its samples, ISO/IEC 14496-12 §8.4.2.
/// * The duration of a track is the sum of the durations of its edits, or,
///   without an edit list, the duration of its media in the timescale of the
///   movie, rounded up, §8.3.2.
/// * Where the last edit of a track is not empty and lasts 0, as the last
///   edit of a fragmented movie may to run to the end of its media
///   (§8.6.6.1), it is closed at the end of the composition of the samples:
///   it lasts from its `media_time` to the latest time a sample of the track
///   is composed until, its decoding time, composition offset and duration
///   added (§8.6.1). The `elst` takes version 1 where that duration does not
///   fit 32 bits.
/// * The duration of the movie is that of its longest track, §8.2.2.
/// * The `mvhd`, each `tkhd` and each `mdhd` are of version 0 where their
///   duration, `creation_time` and `modification_time` fit 32 bits, and of
///   version 1 otherwise.
///
/// The other fields, the `next_track_ID` among them, are left as they are.
///
/// ```
/// use isobmff::boxes::{MovieBox, TimeToSampleBox, TimeToSampleEntry, TrackBox};
/// use isobmff::structure_data::set_durations;
///
/// // A track of media in 90,000 units a second, of 3 samples of 3,000 each
/// let mut trak = TrackBox::default();
/// trak.mdia.mdhd.timescale = 90_000;
/// let mut run = TimeToSampleEntry::default();
/// (run.sample_count, run.sample_delta) = (3, 3_000);
/// let mut stts = TimeToSampleBox::default();
/// stts.entries = vec![run];
/// trak.mdia.minf.stbl.stts = Some(stts);
/// let mut moov = MovieBox::default();
/// moov.mvhd.timescale = 1_000;
/// moov.trak = vec![trak];
///
/// set_durations(&mut moov);
/// assert_eq!((moov.mvhd.duration, moov.trak[0].mdia.mdhd.duration), (100, 9_000));
/// ```
pub fn set_durations(movie: &mut MovieBox) {
    let movie_timescale = movie.mvhd.timescale;
    for trak in &mut movie.trak {
        let stbl = &trak.mdia.minf.stbl;
        let media_duration = stbl.stts.as_ref().map_or(0, |stts| {
            stts.entries.iter().fold(0_u64, |total, entry| {
                total.saturating_add(
                    u64::from(entry.sample_count).saturating_mul(u64::from(entry.sample_delta)),
                )
            })
        });
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
        let composed = composition_end(stbl);
        let elst = trak.edts.as_mut().and_then(|edts| edts.elst.as_mut());
        if let Some(elst) = elst {
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
            trak.tkhd.duration = in_movie_timescale(media_duration);
        }
        let tkhd = &mut trak.tkhd;
        tkhd.version = version_for([tkhd.duration, tkhd.creation_time, tkhd.modification_time]);
        let mdhd = &mut trak.mdia.mdhd;
        mdhd.duration = media_duration;
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

/// Returns the movie extends box of the tracks `trak`: a track extends box for each, whose default sample entry is the first
///
/// The `movie extends` box, [`MovieExtendsBox`] (`mvex`), ISO/IEC 14496-12
/// §8.8.1, which says the movie goes on in movie fragments; and a
/// [`TrackExtendsBox`] (`trex`), §8.8.3, for each track, with no other
/// default.
///
/// ```
/// use isobmff::boxes::TrackBox;
/// use isobmff::structure_data::movie_extends_box;
///
/// let mut trak = TrackBox::default();
/// trak.tkhd.track_id = 1;
/// let mvex = movie_extends_box(&[trak]);
/// assert_eq!(mvex.trex.iter().map(|trex| trex.track_id).collect::<Vec<_>>(), [1]);
/// ```
#[must_use]
pub fn movie_extends_box(trak: &[TrackBox]) -> MovieExtendsBox {
    let mut mvex = MovieExtendsBox::default();
    mvex.trex = trak
        .iter()
        .map(|trak| {
            let mut trex = TrackExtendsBox::default();
            trex.track_id = trak.tkhd.track_id;
            trex.default_sample_description_index = 1;
            trex
        })
        .collect();
    mvex
}

/// The latest time a sample of `stbl` is composed until, its decoding time, composition offset and duration added, in the timescale of the media, §8.6.1
fn composition_end(stbl: &SampleTableBox) -> u64 {
    let mut offsets = stbl
        .ctts
        .iter()
        .flat_map(|ctts| ctts.entries.iter())
        .map(|entry| (entry.sample_count, entry.sample_offset));
    let (mut offset_count, mut offset) = offsets.next().unwrap_or((0, 0));
    let (mut decoding_time, mut end) = (0_i128, 0_i128);
    for run in stbl.stts.iter().flat_map(|stts| stts.entries.iter()) {
        let (mut count, delta) = (run.sample_count, i128::from(run.sample_delta));
        // Why not look at each sample: within a run of one duration and one
        // composition offset, each sample ends later than the one before, so
        // the last of the run ends latest, and the walk is linear in runs.
        while count > 0 {
            while offset_count == 0 {
                (offset_count, offset) = offsets.next().unwrap_or((u32::MAX, 0));
            }
            let taken = count.min(offset_count);
            decoding_time = decoding_time.saturating_add(delta.saturating_mul(i128::from(taken)));
            end = end.max(decoding_time.saturating_add(i128::from(offset)));
            count = count.saturating_sub(taken);
            offset_count = offset_count.saturating_sub(taken);
        }
    }
    u64::try_from(end.max(0)).unwrap_or(u64::MAX)
}

/// The version of a box whose `values` take 32 bits in version 0 and 64 in version 1
fn version_for(values: [u64; 3]) -> u8 {
    u8::from(values.iter().any(|&value| u32::try_from(value).is_err()))
}
