//! [`sample_extents`], the samples the sample tables of a movie declare resolved to where they lie, ISO/IEC 14496-12 §8.5.1 and §8.7

use alloc::vec::Vec;

use isobmff_boxes::{MovieBox, SampleFlags, TrackBox};

use crate::composition_time_offset;
use crate::error::Error;
use crate::sample::SampleExtent;
use crate::sample_description::SampleDescriptions;

/// Resolves the samples the sample tables of `movie` declare, in the order their bytes lie in the file
///
/// The `stbl` of each track (ISO/IEC 14496-12 §8.5.1) spreads what it declares
/// about a sample over four tables, and a sample is read across them: the
/// `stts` states when it is decoded, as a delta from the sample before it
/// summed from zero (§8.6.1.2); the `stsc` states which chunk it lies in and
/// which `stsd` entry describes it, by runs of chunks holding the same number
/// of samples (§8.7.4); the `stsz` or the `stz2` states how many bytes it
/// occupies (§8.7.3); and the `stco` or the `co64` states where its chunk starts in
/// the file, the samples of a chunk lying one after another from there
/// (§8.7.5). The `data_reference_index` of each sample is read off the `stsd`
/// entry that describes it (§8.5.2.3), which has to name the file itself.
///
/// Five optional tables state the rest: the `ctts`
/// states the composition time offset (§8.6.1.3), and the `sdtp` (§8.6.4),
/// the `padb` (§8.7.6), the `stss` (§8.6.2) and the `stdp` (§8.5.3) state the
/// fields of the `sample_flags` laid out as §8.8.3.1 lays them out in a movie
/// fragment. A track carrying none of them has every sample composed when it
/// is decoded and every sample a sync sample, so its samples come out with a
/// composition time offset of zero and [`SampleFlags::ZERO`], and a table
/// missing on its own leaves its fields zero, or the sample a sync sample for
/// the `stss`. A track declaring no sample — one carried in fragments —
/// contributes nothing, and a chunk its `stsc` lays no run over holds none.
///
/// The samples are counted before one is laid out: the sample size tables of
/// every track together counting more than `sample_count_limit` lay out none.
///
/// The extents of every track come out together in the order their bytes lie
/// in the file, the samples of one chunk in sample order and the chunks of
/// one track between those of another where the file interleaves them, which
/// is the order a reader fed the file from its start meets them in. They stop
/// at the first failure, which is the last item. They borrow nothing: the
/// movie is the caller's again once the call returns.
///
/// # Errors
///
/// Returned as the last of the extents:
///
/// * [`SampleCountLimitExceeded`](crate::ErrorKind::SampleCountLimitExceeded):
///   the `stsz` or `stz2` of every track together count more samples than
///   `sample_count_limit`, and no extent comes before it.
/// * [`SampleCountMismatch`](crate::ErrorKind::SampleCountMismatch):
///   the tables of a track count different numbers of samples.
/// * [`SyncSampleOutOfRange`](crate::ErrorKind::SyncSampleOutOfRange):
///   the `stss` of a track lists a sample number no later than the one
///   before it, or outside the samples of the track (0 or past the last).
/// * [`FirstChunkOutOfRange`](crate::ErrorKind::FirstChunkOutOfRange):
///   a run of chunks of a track starts at a chunk outside the range open to it.
/// * [`UnknownSampleDescriptionIndex`](crate::ErrorKind::UnknownSampleDescriptionIndex):
///   a run describes its samples by an `stsd` entry its track has none of.
/// * The failures of [`SampleEntry::try_from`](isobmff_boxes::SampleEntry),
///   carried on [`Box`](crate::ErrorKind::Box): the `stsd` entry does
///   not read as a sample entry, with `stsd` added to the containers.
/// * [`UnknownDataReferenceIndex`](crate::ErrorKind::UnknownDataReferenceIndex):
///   the `stsd` entry names a `dref` entry its track has none of.
/// * [`ExternalDataReference`](crate::ErrorKind::ExternalDataReference):
///   the `dref` entry names a resource other than the file itself.
/// * [`DecodeTimeOverflow`](crate::ErrorKind::DecodeTimeOverflow): the
///   decode times of a track run past what 64 bits carry.
/// * [`DataOffsetOverflow`](crate::ErrorKind::DataOffsetOverflow): the
///   offsets of a track run past what 64 bits carry.
pub fn sample_extents(
    movie: &MovieBox,
    sample_count_limit: u64,
) -> impl Iterator<Item = Result<SampleExtent, Error>> + use<> {
    let declared = movie
        .trak()
        .iter()
        .map(|trak| trak.mdia().minf().stbl().sample_sizes().sample_count())
        .fold(0, u64::saturating_add);
    let mut extents = Vec::new();
    let outcome = if declared > sample_count_limit {
        Err(Error::sample_count_limit_exceeded(
            declared,
            sample_count_limit,
        ))
    } else {
        movie
            .trak()
            .iter()
            .try_for_each(|trak| resolve_track(trak, &mut extents))
    };
    extents.sort_by_key(|extent| extent.extent().start);

    extents.into_iter().map(Ok).chain(outcome.err().map(Err))
}

/// Resolves the samples the sample table of `trak` declares into `extents`, in sample order
fn resolve_track(trak: &TrackBox, extents: &mut Vec<SampleExtent>) -> Result<(), Error> {
    let track_id = trak.tkhd().track_id();
    let stbl = trak.mdia().minf().stbl();
    let descriptions = SampleDescriptions::new(trak);
    let mut sizes = stbl.sample_sizes().sizes();
    let mut deltas = stbl.stts().deltas();
    let mut runs = stbl.stsc().entries().iter().peekable();
    if let Some(first) = runs.peek().filter(|run| run.first_chunk() != 1) {
        return Err(Error::first_chunk_out_of_range(
            track_id,
            first.first_chunk(),
        ));
    }
    let mut offsets = stbl
        .ctts()
        .map(|ctts| ctts.offsets().map(composition_time_offset::resolved));
    let mut dependencies = stbl.sdtp().map(|sdtp| sdtp.entries().iter().copied());
    let mut paddings = stbl.padb().map(|padb| padb.entries().iter().copied());
    let mut priorities = stbl.stdp().map(|stdp| stdp.entries().iter().copied());
    let mut sync_samples = stbl.stss().map(|stss| stss.entries().iter().peekable());
    let mut active_run = None;
    let mut decode_time = 0_u64;
    let mut sample_number = 0_u64;

    for (chunk, chunk_offset) in (1_u64..).zip(stbl.chunk_offsets().offsets()) {
        if let Some(run) = runs.next_if(|run| u64::from(run.first_chunk()) == chunk) {
            let data_reference_index =
                descriptions.data_reference_index(run.sample_description_index())?;
            active_run = Some((run, data_reference_index));
        }
        let Some((run, data_reference_index)) = active_run else {
            continue;
        };
        let mut data_offset = chunk_offset;

        for _ in 0..run.samples_per_chunk() {
            let (
                Some(size),
                Some(delta),
                Some(offset),
                Some(dependency),
                Some(padding),
                Some(degradation_priority),
            ) = (
                sizes.next(),
                deltas.next(),
                next_stated(&mut offsets),
                next_stated(&mut dependencies),
                next_stated(&mut paddings),
                next_stated(&mut priorities),
            )
            else {
                return Err(Error::sample_count_mismatch(track_id));
            };
            sample_number = sample_number.saturating_add(1);
            let is_sync_sample = sync_samples.as_mut().is_none_or(|listed| {
                listed
                    .next_if(|entry| u64::from(entry.sample_number()) == sample_number)
                    .is_some()
            });
            let sample_flags =
                SampleFlags::new(dependency, padding, !is_sync_sample, degradation_priority);
            let data_end = data_offset
                .checked_add(u64::from(size))
                .ok_or(Error::data_offset_overflow(track_id))?;

            extents.push(SampleExtent::new(
                track_id,
                decode_time,
                delta,
                offset,
                sample_flags,
                run.sample_description_index(),
                data_reference_index,
                data_offset..data_end,
            ));

            decode_time = decode_time
                .checked_add(u64::from(delta))
                .ok_or(Error::decode_time_overflow(track_id))?;
            data_offset = data_end;
        }
    }
    if let Some(run) = runs.next() {
        return Err(Error::first_chunk_out_of_range(track_id, run.first_chunk()));
    }
    if sizes.next().is_some()
        || deltas.next().is_some()
        || offsets.as_mut().and_then(Iterator::next).is_some()
        || dependencies.as_mut().and_then(Iterator::next).is_some()
        || paddings.as_mut().and_then(Iterator::next).is_some()
        || priorities.as_mut().and_then(Iterator::next).is_some()
    {
        return Err(Error::sample_count_mismatch(track_id));
    }
    if let Some(entry) = sync_samples.as_mut().and_then(Iterator::next) {
        return Err(Error::sync_sample_out_of_range(
            track_id,
            entry.sample_number(),
        ));
    }

    Ok(())
}

/// Returns what the table states for the next sample, or its default when the track carries no such table
///
/// `None` when the table has run out of samples.
fn next_stated<Entry: Default>(table: &mut Option<impl Iterator<Item = Entry>>) -> Option<Entry> {
    match table {
        Some(entries) => entries.next(),
        None => Some(Entry::default()),
    }
}

#[cfg(test)]
mod tests;
