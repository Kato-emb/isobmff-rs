//! The samples the sample table of a track declares, read across its tables, ISO/IEC 14496-12 §8.5.1 and §8.7

use alloc::vec::Vec;
use core::iter;

use isobmff_boxes::{
    ChunkOffsetVariant, CompositionOffsetBox, SampleDependencyTypeBox, SampleFlags,
    SampleSizeVariant, SampleSizes, SampleTableBox, SampleToChunkBox, SyncSampleBox,
    TimeToSampleBox, TrackBox,
};
use isobmff_core::BoxDefinition;

use super::{data_reference_index, expand};
use crate::error::DemuxError;
use crate::framing::to_u64;
use crate::sample::{SampleLocation, SampleProperties};

/// Returns the number of samples the sample size table of `trak` counts
pub(super) fn sample_count(trak: &TrackBox) -> u64 {
    let (_, constant_count, table) = size_table(&trak.mdia.minf.stbl);

    u64::from(constant_count).saturating_add(to_u64(table.len()))
}

/// Resolves the samples the sample table of `trak` declares into `locations`, in sample order, and returns the decoding time past the last of them
///
/// The `stbl` spreads what it declares about a sample over its tables, which
/// are read side by side: the `stts` gives its duration, its decoding time
/// being the sum of the durations before it (§8.6.1.2); the `ctts` its
/// composition time offset (§8.6.1.3); the `stsc` the chunk it lies in and
/// the sample entry that describes it, by runs of chunks (§8.7.4); the `stsz`
/// or the `stz2` its size (§8.7.3); and the `stco` or the `co64` where its
/// chunk starts, the samples of a chunk lying one after another from there
/// (§8.7.5). The `stss` marks the sync samples, every sample being one where
/// the track has no `stss` (§8.6.2), and the `sdtp` gives the dependencies of
/// each (§8.6.4), which make up the `sample_flags` as §8.8.3.1 lays them out.
/// A missing table declares no sample, and a missing optional one leaves its
/// fields zero.
///
/// # Errors
///
/// * [`SampleTableMismatch`](DemuxError::SampleTableMismatch): a table counts
///   a different number of samples than the `stsz` or the `stz2`, the `stsc`
///   lays its runs over chunks the `stco` or the `co64` does not have, or the
///   `stss` marks a sample number out of order or past the last sample.
/// * The failures of the sample entry and the data reference entry each run
///   of chunks names.
/// * [`DataOffsetOutOfRange`](DemuxError::DataOffsetOutOfRange),
///   [`DecodingTimeOverflow`](DemuxError::DecodingTimeOverflow): the offsets or
///   the decoding times run past 64 bits.
pub(super) fn resolve_track(
    trak: &TrackBox,
    locations: &mut Vec<SampleLocation>,
) -> Result<u64, DemuxError> {
    let track_id = trak.tkhd.track_id;
    let stbl = &trak.mdia.minf.stbl;
    let mismatch = |box_type| DemuxError::SampleTableMismatch { track_id, box_type };

    let mut sizes = sample_sizes(stbl);
    let mut durations = expand(stbl.stts.iter().flat_map(|stts| {
        stts.entries
            .iter()
            .map(|entry| (entry.sample_count, entry.sample_delta))
    }));
    let mut composition_offsets = stbl.ctts.as_ref().map(|ctts| {
        expand(
            ctts.entries
                .iter()
                .map(|entry| (entry.sample_count, entry.sample_offset)),
        )
    });
    let mut dependencies = stbl.sdtp.as_ref().map(|sdtp| sdtp.entries.iter().copied());
    let mut sync_samples = stbl
        .stss
        .as_ref()
        .map(|stss| stss.sample_numbers.iter().copied().peekable());
    let mut runs = stbl.stsc.iter().flat_map(|stsc| &stsc.entries).peekable();

    let mut run = None;
    let mut decoding_time = 0_u64;
    let mut sample_number = 0_u64;
    for (chunk, chunk_offset) in (1_u32..).zip(chunk_offsets(stbl)) {
        if let Some(next) = runs.next_if(|next| next.first_chunk == chunk) {
            let data_reference_index = data_reference_index(trak, next.sample_description_index)?;
            run = Some((next, data_reference_index));
        }
        let Some((run, data_reference_index)) = run else {
            // Chunks before the first run hold no sample.
            return Err(mismatch(SampleToChunkBox::BOX_TYPE));
        };

        let mut data_offset = chunk_offset;
        for _ in 0..run.samples_per_chunk {
            let size = sizes
                .next()
                .ok_or_else(|| mismatch(SampleToChunkBox::BOX_TYPE))?;
            let sample_duration = durations
                .next()
                .ok_or_else(|| mismatch(TimeToSampleBox::BOX_TYPE))?;
            let sample_composition_time_offset = next_or_zero(&mut composition_offsets)
                .ok_or_else(|| mismatch(CompositionOffsetBox::BOX_TYPE))?;
            let dependency = next_or_zero(&mut dependencies)
                .ok_or_else(|| mismatch(SampleDependencyTypeBox::BOX_TYPE))?;
            sample_number = sample_number.saturating_add(1);
            let is_sync_sample = sync_samples.as_mut().is_none_or(|marked| {
                marked
                    .next_if(|marked| u64::from(*marked) == sample_number)
                    .is_some()
            });
            if sync_samples
                .as_mut()
                .and_then(|marked| marked.peek())
                .is_some_and(|marked| u64::from(*marked) <= sample_number)
            {
                return Err(mismatch(SyncSampleBox::BOX_TYPE));
            }

            let data_end = data_offset
                .checked_add(u64::from(size))
                .ok_or(DemuxError::DataOffsetOutOfRange { track_id })?;
            locations.push(SampleLocation::new(
                SampleProperties {
                    track_id,
                    decoding_time,
                    sample_duration,
                    sample_composition_time_offset,
                    sample_flags: {
                        let mut flags = SampleFlags::default();
                        flags.is_leading = dependency.is_leading;
                        flags.sample_depends_on = dependency.sample_depends_on;
                        flags.sample_is_depended_on = dependency.sample_is_depended_on;
                        flags.sample_has_redundancy = dependency.sample_has_redundancy;
                        flags.sample_is_non_sync_sample = !is_sync_sample;
                        flags
                    },
                    sample_description_index: run.sample_description_index,
                },
                data_reference_index,
                data_offset..data_end,
            ));
            decoding_time = decoding_time
                .checked_add(u64::from(sample_duration))
                .ok_or(DemuxError::DecodingTimeOverflow { track_id })?;
            data_offset = data_end;
        }
    }

    let left_over = [
        (
            runs.next().is_some() || sizes.next().is_some(),
            SampleToChunkBox::BOX_TYPE,
        ),
        (durations.next().is_some(), TimeToSampleBox::BOX_TYPE),
        (
            composition_offsets
                .and_then(|mut table| table.next())
                .is_some(),
            CompositionOffsetBox::BOX_TYPE,
        ),
        (
            dependencies.and_then(|mut table| table.next()).is_some(),
            SampleDependencyTypeBox::BOX_TYPE,
        ),
        (
            sync_samples.and_then(|mut table| table.next()).is_some(),
            SyncSampleBox::BOX_TYPE,
        ),
    ];
    match left_over.into_iter().find(|(left, _)| *left) {
        Some((_, box_type)) => Err(mismatch(box_type)),
        None => Ok(decoding_time),
    }
}

/// Returns the size of each sample the `stsz` or the `stz2` states, in sample order
fn sample_sizes(stbl: &SampleTableBox) -> impl Iterator<Item = u32> {
    let (constant_size, constant_count, table) = size_table(stbl);

    expand(iter::once((constant_count, constant_size))).chain(table.iter().copied())
}

/// Returns the sizes the `stsz` or the `stz2` states: a size and the number of samples of it, or a table of a size per sample
///
/// A sample size table of a form the reader does not know declares no sample.
fn size_table(stbl: &SampleTableBox) -> (u32, u32, &[u32]) {
    match &stbl.sample_size {
        Some(SampleSizeVariant::SampleSize(stsz)) => match &stsz.sample_sizes {
            SampleSizes::Constant {
                sample_size,
                sample_count,
            } => (sample_size.get(), *sample_count, &[]),
            SampleSizes::Table { entry_sizes } => (0, 0, entry_sizes),
            _ => (0, 0, &[]),
        },
        Some(SampleSizeVariant::CompactSampleSize(stz2)) => (0, 0, &stz2.entry_sizes),
        Some(_) | None => (0, 0, &[]),
    }
}

/// Returns the file offset of each chunk the `stco` or the `co64` states, in chunk order
fn chunk_offsets(stbl: &SampleTableBox) -> impl Iterator<Item = u64> {
    let (compact, large): (&[u32], &[u64]) = match &stbl.chunk_offset {
        Some(ChunkOffsetVariant::ChunkOffset(stco)) => (&stco.chunk_offsets, &[]),
        Some(ChunkOffsetVariant::ChunkLargeOffset(co64)) => (&[], &co64.chunk_offsets),
        Some(_) | None => (&[], &[]),
    };

    compact
        .iter()
        .copied()
        .map(u64::from)
        .chain(large.iter().copied())
}

/// Returns what the table states for the next sample, or zero when the track has no such table
///
/// `None` when the table has run out of samples.
fn next_or_zero<Entry: Default>(table: &mut Option<impl Iterator<Item = Entry>>) -> Option<Entry> {
    match table {
        Some(entries) => entries.next(),
        None => Some(Entry::default()),
    }
}
