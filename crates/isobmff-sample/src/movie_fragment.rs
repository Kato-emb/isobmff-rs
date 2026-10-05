//! [`sample_extents`], the samples a movie fragment declares resolved against the movie, ISO/IEC 14496-12 §8.8

use alloc::vec::Vec;

use isobmff_boxes::{
    CompositionTimeOffset, MovieBox, MovieFragmentBox, SampleFlags, TrackFragmentBox, TrackRunBox,
};

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
/// `decode_times` is moved to where every track fragment leaves its track —
/// the end of its last sample, or the default duration on from where it
/// started for a fragment declaring an empty duration, which carries no
/// samples (§8.8.7.1) — so the fragment resolved next carries on from there
/// whether or not the extents are taken. A failure returned outright leaves
/// `decode_times` as it was. A row stating no composition time offset has one
/// of zero. The `data_reference_index` of each sample is read off the `stsd`
/// entry that describes it (§8.5.2.3), which has to name the file itself.
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
///   carries samples of a track the movie declares no `trak` or `trex` for.
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
///   offsets a fragment states run past what 64 bits carry.
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
        .map(|traf| TrackFragment::settle(traf, movie, &mut reached))
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

/// What one track fragment settles for its samples before their data is placed
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

impl TrackFragment {
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
        let (Some(trak), Some(trex)) = (trak, trex) else {
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

        Ok(Self {
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
        })
    }
}

/// Where the samples of a track fragment settle as its runs are walked
///
/// `base` is where the offsets of the track fragment are anchored, which the
/// offset a run states is counted from. `data_offset` is where the sample
/// resolved next starts, and `decode_time` when it is decoded.
struct Cursor {
    base: u64,
    data_offset: u64,
    decode_time: u64,
}

/// Places the data of every track fragment of `movie_fragment` into `extents`, stopping at the first failure
fn resolve_data(
    movie_fragment: &MovieFragmentBox,
    track_fragments: &[TrackFragment],
    moof_start: u64,
    extents: &mut Vec<Result<SampleExtent, Error>>,
) -> Result<(), Error> {
    let mut data_before = None;

    for (traf, settled) in movie_fragment.traf().iter().zip(track_fragments) {
        let tfhd = traf.tfhd();
        let base = tfhd
            .base_data_offset()
            .unwrap_or(if tfhd.default_base_is_moof() {
                moof_start
            } else {
                data_before.unwrap_or(moof_start)
            });
        let mut cursor = Cursor {
            base,
            data_offset: base,
            decode_time: settled.decode_time,
        };

        for trun in traf.trun() {
            resolve_run(trun, settled, &mut cursor, extents)?;
        }

        data_before = Some(cursor.data_offset);
    }

    Ok(())
}

/// Resolves the samples `trun` declares into `extents`, and moves `cursor` past them
fn resolve_run(
    trun: &TrackRunBox,
    settled: &TrackFragment,
    cursor: &mut Cursor,
    extents: &mut Vec<Result<SampleExtent, Error>>,
) -> Result<(), Error> {
    let track_id = settled.track_id;
    if let Some(stated) = trun.data_offset() {
        cursor.data_offset = cursor
            .base
            .checked_add_signed(i64::from(stated))
            .ok_or(Error::data_offset_overflow(track_id))?;
    }

    let mut first_sample_flags = trun.first_sample_flags();
    for row in trun.samples() {
        let declared = u64::from(row.sample_size().unwrap_or(settled.sample_size));
        let data_end = cursor
            .data_offset
            .checked_add(declared)
            .ok_or(Error::data_offset_overflow(track_id))?;
        let sample_duration = row.sample_duration().unwrap_or(settled.sample_duration);

        extents.push(Ok(SampleExtent::new(
            track_id,
            cursor.decode_time,
            sample_duration,
            row.sample_composition_time_offset()
                .map_or(0, CompositionTimeOffset::get),
            first_sample_flags
                .take()
                .or(row.sample_flags())
                .unwrap_or(settled.sample_flags),
            settled.sample_description_index,
            settled.data_reference_index,
            cursor.data_offset..data_end,
        )));

        // Why not checked_add: TrackFragment::settle summed these same durations
        // from the same start and refused the fragment on overflow, so this
        // cannot wrap.
        cursor.decode_time = cursor.decode_time.wrapping_add(u64::from(sample_duration));
        cursor.data_offset = data_end;
    }

    Ok(())
}

#[cfg(test)]
mod tests;
