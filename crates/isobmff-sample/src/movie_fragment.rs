//! [`sample_extents`], the samples a movie fragment declares resolved against the movie, ISO/IEC 14496-12 §8.8

use alloc::vec::Vec;
use core::ops::Range;

use isobmff_boxes::{
    MovieBox, MovieFragmentBox, SampleFlags, TrackBox, TrackExtendsBox, TrackFragmentBox,
    TrackRunBox, TrackRunSample,
};
use isobmff_core::BoxDefinition as _;

use crate::composition_time_offset;
use crate::error::Error;
use crate::sample::SampleExtent;
use crate::sample_description::SampleDescriptions;
use crate::track_decode_times::TrackDecodeTimes;

/// Resolves the samples `movie_fragment` declares against `movie`, in the order it declares them
///
/// Each track fragment of the `moof` is resolved by the three-step fallback of
/// ISO/IEC 14496-12 §8.8.7 and §8.8.8: what a `trun` row states, then what
/// the `tfhd` of the fragment states, then what the `trex` of the track states.
/// Where the data of a run lies follows §8.8.7.1: a `tfhd` stating a
/// `base_data_offset` anchors its runs there, one setting
/// `default-base-is-moof` anchors them at `moof_start`, where the `moof`
/// begins in the file, and one stating neither anchors them at the `moof` for
/// the first track fragment and at the end of the data of the track fragment
/// before it for those that follow — which is the anchor of that track
/// fragment if it carried no run. A run stating no `data_offset` starts where
/// the run before it ended, and the first run of a track fragment at its
/// anchor.
///
/// When the samples are decoded follows §8.8.12: a track fragment carrying a
/// `tfdt` starts its samples there, and one carrying none carries on from where
/// `decode_times` has its track, which a `tfdt` settles again for times
/// created by [`TrackDecodeTimes::unknown`]. Before the extents are returned,
/// `decode_times` is moved to where every track fragment of a track the movie
/// reads leaves its track —
/// the end of its last sample, or the default duration on from where it
/// started for a fragment declaring an empty duration, which carries no
/// samples (§8.8.7.1) — so the fragment resolved next carries on from there
/// whether or not the extents are taken. A failure returned outright leaves
/// `decode_times` as it was. A row stating no composition time offset has one
/// of zero. The `data_reference_index` of each sample is read off the `stsd`
/// entry that describes it (§8.5.2.3), which has to name the file itself.
///
/// A movie may keep a `trak` that did not read among its
/// [`other_boxes`](MovieBox::other_boxes). Where it does, a `traf` of a track no
/// `trak` of [`trak`](MovieBox::trak) declares gives no sample and the
/// fragment reads on; its runs are walked only for where its data ends, by the
/// sizes its rows, its `tfhd` or the `trex` of the track state, to anchor a
/// track fragment after it that states no anchor of its own.
///
/// The samples are counted before one is settled: the `trun`s of every track
/// fragment together counting more than `sample_count_limit` settle none.
///
/// The extents come out in the order the fragment declares them, and stop at
/// the first failure, which is the last item. They borrow nothing: the boxes
/// and `decode_times` are the caller's again once the call returns.
///
/// # Errors
///
/// Returned outright, before `decode_times` moves:
///
/// * [`SampleCountLimitExceeded`](crate::ErrorKind::SampleCountLimitExceeded):
///   the `trun`s of every track fragment together count more samples than
///   `sample_count_limit`.
/// * [`MissingMovieExtends`](crate::ErrorKind::MissingMovieExtends): a
///   `traf` continues a movie that carries no `mvex`, and so no fragments.
/// * [`UnknownTrackId`](crate::ErrorKind::UnknownTrackId): a `traf`
///   carries samples of a track no read `trak` declares and the movie keeps no
///   `trak` unread, or of a track the movie reads but declares no `trex` for.
/// * [`UnknownSampleDescriptionIndex`](crate::ErrorKind::UnknownSampleDescriptionIndex):
///   a `traf` describes its samples by an `stsd` entry its track has none of.
/// * The failures of [`SampleEntry::try_from`](isobmff_boxes::SampleEntry),
///   carried on [`Box`](crate::ErrorKind::Box): the `stsd` entry does
///   not read as a sample entry, with `stsd` added to the containers.
/// * [`UnknownDataReferenceIndex`](crate::ErrorKind::UnknownDataReferenceIndex):
///   the `stsd` entry names a `dref` entry its track has none of.
/// * [`ExternalDataReference`](crate::ErrorKind::ExternalDataReference):
///   the `dref` entry names a resource other than the file itself.
/// * [`MissingDecodeTime`](crate::ErrorKind::MissingDecodeTime): a `traf`
///   carries no `tfdt`, and `decode_times` does not know where its track
///   stands.
/// * [`DecodeTimeOverflow`](crate::ErrorKind::DecodeTimeOverflow): the
///   decode times of a track run past what 64 bits carry.
///
/// Returned as the last of the extents:
///
/// * [`DataOffsetOverflow`](crate::ErrorKind::DataOffsetOverflow): the
///   offsets a `traf` of a track the movie reads states run past what 64 bits
///   carry.
///
/// Returned as the last of the extents, naming a track kept unread, where the
/// end of the data of a `traf` of that track is unknown and a `traf` of a track
/// the movie reads is anchored there, through any `traf`s between them that
/// state no anchor; where no such `traf` follows, the fragment reads on. The
/// end is unknown where, in a run with no run after it stating a
/// `data_offset`:
///
/// * [`UnknownTrackId`](crate::ErrorKind::UnknownTrackId): a row states no
///   size, and neither the `tfhd` of the `traf` nor the `trex` of its track
///   states one.
/// * [`DataOffsetOverflow`](crate::ErrorKind::DataOffsetOverflow): the offsets
///   run past what 64 bits carry.
pub fn sample_extents(
    movie_fragment: &MovieFragmentBox,
    movie: &MovieBox,
    moof_start: u64,
    decode_times: &mut TrackDecodeTimes,
    sample_count_limit: u64,
) -> Result<impl Iterator<Item = Result<SampleExtent, Error>> + use<>, Error> {
    let declared = movie_fragment
        .traf()
        .iter()
        .flat_map(|traf| traf.trun())
        .map(|trun| u64::from(trun.sample_count()))
        .fold(0, u64::saturating_add);
    if declared > sample_count_limit {
        return Err(Error::sample_count_limit_exceeded(
            declared,
            sample_count_limit,
        ));
    }

    let mut reached = decode_times.clone();
    let track_fragments = movie_fragment
        .traf()
        .iter()
        .map(|traf| SettledFragment::settle(traf, movie, &mut reached))
        .collect::<Result<Vec<_>, _>>()?;
    *decode_times = reached;

    // Why not chaining the failure after an iterator of the extents: the
    // chained iterator costs a reader ten nanoseconds an extent over a plain
    // one, a fifth of what reading a small sample costs in all.
    let mut extents = Vec::with_capacity(usize::try_from(declared.saturating_add(1)).unwrap_or(0));
    let outcome = resolve_data(movie_fragment, &track_fragments, moof_start, &mut extents);
    extents.extend(outcome.err().map(Err));

    Ok(extents.into_iter())
}

/// What one track fragment settles before its data is placed
enum SettledFragment {
    /// A track fragment of a track the movie reads, whose samples are resolved
    Read(TrackFragment),
    /// A track fragment of a track the movie kept unread, which gives no sample
    ///
    /// `sample_size` is the size its runs fall back on, where the `tfhd` or the
    /// `trex` of the track states one; the data of its runs is walked only to
    /// anchor a track fragment after it.
    KeptUnread { sample_size: Option<u32> },
}

/// What one track fragment of a track the movie reads settles for its samples before their data is placed
///
/// The defaults are what a `tfhd` states for the fragment where it carries one
/// and the `trex` of the track states where it does not (ISO/IEC 14496-12
/// §8.8.7); `decode_time` is when the first sample of the fragment is decoded.
struct TrackFragment {
    track_id: u32,
    sample_description_index: u32,
    sample_duration: u32,
    sample_size: u32,
    sample_flags: SampleFlags,
    data_reference_index: u16,
    decode_time: u64,
}

impl SettledFragment {
    /// Settles `traf` against `movie`, and moves `reached` past its samples, ISO/IEC 14496-12 §8.8.12
    fn settle(
        traf: &TrackFragmentBox,
        movie: &MovieBox,
        reached: &mut TrackDecodeTimes,
    ) -> Result<Self, Error> {
        let Some(mvex) = movie.mvex() else {
            return Err(Error::missing_movie_extends());
        };
        let tfhd = traf.tfhd();
        let track_id = tfhd.track_id();
        let trak = movie
            .trak()
            .iter()
            .find(|trak| trak.tkhd().track_id() == track_id);
        let trex = mvex.trex().iter().find(|trex| trex.track_id() == track_id);
        let Some(trak) = trak else {
            let keeps_a_track_unread = movie
                .other_boxes()
                .iter()
                .any(|kept| kept.box_type() == TrackBox::BOX_TYPE);
            if !keeps_a_track_unread {
                return Err(Error::unknown_track_id(track_id));
            }

            return Ok(Self::KeptUnread {
                sample_size: tfhd
                    .default_sample_size()
                    .or(trex.map(TrackExtendsBox::default_sample_size)),
            });
        };
        let Some(trex) = trex else {
            return Err(Error::unknown_track_id(track_id));
        };

        let sample_description_index = tfhd
            .sample_description_index()
            .unwrap_or(trex.default_sample_description_index());
        let sample_duration = tfhd
            .default_sample_duration()
            .unwrap_or(trex.default_sample_duration());
        let data_reference_index =
            SampleDescriptions::new(trak).data_reference_index(sample_description_index)?;
        let decode_time = match traf.tfdt() {
            Some(tfdt) => tfdt.base_media_decode_time(),
            None => reached
                .decode_time(track_id)
                .ok_or(Error::missing_decode_time(track_id))?,
        };

        let overflow = || Error::decode_time_overflow(track_id);
        let mut end = decode_time;
        if tfhd.duration_is_empty() {
            end = end
                .checked_add(u64::from(sample_duration))
                .ok_or_else(overflow)?;
        }
        for row in traf.trun().iter().flat_map(TrackRunBox::samples) {
            let lasts = u64::from(row.sample_duration().unwrap_or(sample_duration));
            end = end.checked_add(lasts).ok_or_else(overflow)?;
        }
        reached.reach(track_id, end);

        Ok(Self::Read(TrackFragment {
            track_id,
            sample_description_index,
            sample_duration,
            sample_size: tfhd
                .default_sample_size()
                .unwrap_or(trex.default_sample_size()),
            sample_flags: tfhd
                .default_sample_flags()
                .unwrap_or(trex.default_sample_flags()),
            data_reference_index,
            decode_time,
        }))
    }
}

/// Places the data of every track fragment of `movie_fragment` into `extents`, stopping at the first failure
fn resolve_data(
    movie_fragment: &MovieFragmentBox,
    track_fragments: &[SettledFragment],
    moof_start: u64,
    extents: &mut Vec<Result<SampleExtent, Error>>,
) -> Result<(), Error> {
    let mut data_before: Result<u64, Error> = Ok(moof_start);

    for (traf, settled) in movie_fragment.traf().iter().zip(track_fragments) {
        let tfhd = traf.tfhd();
        let base = match tfhd.base_data_offset() {
            Some(base) => Ok(base),
            None if tfhd.default_base_is_moof() => Ok(moof_start),
            None => data_before,
        };

        data_before = match settled {
            SettledFragment::Read(settled) => {
                let base = base?;
                let mut data_offset = base;
                let mut decode_time = settled.decode_time;
                for trun in traf.trun() {
                    let mut first_sample_flags = trun.first_sample_flags();
                    place_run(
                        trun,
                        base,
                        &mut data_offset,
                        Some(settled.sample_size),
                        settled.track_id,
                        |row, data| {
                            let sample_duration =
                                row.sample_duration().unwrap_or(settled.sample_duration);
                            extents.push(Ok(SampleExtent::new(
                                settled.track_id,
                                decode_time,
                                sample_duration,
                                row.sample_composition_time_offset()
                                    .map_or(0, composition_time_offset::resolved),
                                first_sample_flags
                                    .take()
                                    .or(row.sample_flags())
                                    .unwrap_or(settled.sample_flags),
                                settled.sample_description_index,
                                settled.data_reference_index,
                                data,
                            )));

                            // Why not checked_add: SettledFragment::settle summed these same
                            // durations from the same start and refused the fragment on
                            // overflow, so this cannot wrap.
                            decode_time = decode_time.wrapping_add(u64::from(sample_duration));
                        },
                    )?;
                }

                Ok(data_offset)
            }
            SettledFragment::KeptUnread { sample_size } => {
                let mut data_end = base;
                for trun in traf.trun() {
                    let start = if trun.data_offset().is_some() {
                        base
                    } else {
                        data_end
                    };
                    data_end = base.and_then(|base| {
                        let mut data_offset = start?;
                        place_run(
                            trun,
                            base,
                            &mut data_offset,
                            *sample_size,
                            tfhd.track_id(),
                            |_, _| {},
                        )?;

                        Ok(data_offset)
                    });
                }

                data_end
            }
        };
    }

    Ok(())
}

/// Walks the rows of `trun`, handing `place` each row and the data it occupies, and moves `data_offset` past them
///
/// A run stating a `data_offset` starts that far from `base`, the anchor of its
/// track fragment, and one stating none where `data_offset` stands (ISO/IEC
/// 14496-12 §8.8.8.3). A row stating no size takes `sample_size`.
///
/// # Errors
///
/// * [`DataOffsetOverflow`](crate::ErrorKind::DataOffsetOverflow): the
///   offsets run past what 64 bits carry.
/// * [`UnknownTrackId`](crate::ErrorKind::UnknownTrackId): a row states no
///   size and `sample_size` is `None`.
fn place_run(
    trun: &TrackRunBox,
    base: u64,
    data_offset: &mut u64,
    sample_size: Option<u32>,
    track_id: u32,
    mut place: impl FnMut(TrackRunSample, Range<u64>),
) -> Result<(), Error> {
    if let Some(stated) = trun.data_offset() {
        *data_offset = base
            .checked_add_signed(i64::from(stated))
            .ok_or(Error::data_offset_overflow(track_id))?;
    }

    for row in trun.samples() {
        let size = row
            .sample_size()
            .or(sample_size)
            .ok_or(Error::unknown_track_id(track_id))?;
        let data_end = data_offset
            .checked_add(u64::from(size))
            .ok_or(Error::data_offset_overflow(track_id))?;
        place(row, *data_offset..data_end);
        *data_offset = data_end;
    }

    Ok(())
}

#[cfg(test)]
mod tests;
