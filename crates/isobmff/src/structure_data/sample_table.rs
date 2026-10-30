//! The sample table of a track, ISO/IEC 14496-12 §8.5.1 and §8.7: the samples its tables declare, read across them, and the tables written of the samples a writer adds

use alloc::vec::Vec;
use core::iter;
use core::num::NonZeroU32;

use isobmff_boxes::{
    ChunkLargeOffsetBox, ChunkOffsetBox, ChunkOffsetVariant, CompactSampleToGroupBox,
    CompositionOffsetBox, CompositionOffsetEntry, DegradationPriorityBox, MovieBox, PaddingBitsBox,
    SampleAuxiliaryInformationOffsetsBox, SampleAuxiliaryInformationSizesBox,
    SampleDependencyTypeBox, SampleDependencyTypeEntry, SampleFlags, SampleInfoSizes,
    SampleSizeBox, SampleSizeVariant, SampleSizes, SampleTableBox, SampleToChunkBox,
    SampleToChunkEntry, SampleToGroupBox, ShadowSyncSampleBox, SubSampleInformationBox,
    SubSampleReferenceTableBox, SyncSampleBox, TimeToSampleBox, TimeToSampleEntry, TrackBox,
};
use isobmff_core::BoxDefinition;

use super::sample_description::data_reference_index;
use crate::error::{DemuxError, MuxError};
use crate::framing::to_u64;
use crate::sample::{Sample, SampleLocation, SampleProperties};

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
/// each (§8.6.4), the `padb` the bits it is padded with (§8.7.6) and the
/// `stdp` its degradation priority (§8.5.3), which make up the `sample_flags`
/// as §8.8.3.1 lays them out. A missing table declares no sample, and a
/// missing optional one leaves its fields zero.
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
    let mut paddings = stbl.padb.as_ref().map(|padb| padb.pads.iter().copied());
    let mut priorities = stbl
        .stdp
        .as_ref()
        .map(|stdp| stdp.priorities.iter().copied());
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
            let sample_padding_value =
                next_or_zero(&mut paddings).ok_or_else(|| mismatch(PaddingBitsBox::BOX_TYPE))?;
            let sample_degradation_priority = next_or_zero(&mut priorities)
                .ok_or_else(|| mismatch(DegradationPriorityBox::BOX_TYPE))?;
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
                        flags.sample_padding_value = sample_padding_value;
                        flags.sample_is_non_sync_sample = !is_sync_sample;
                        flags.sample_degradation_priority = sample_degradation_priority;
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
            paddings.and_then(|mut table| table.next()).is_some(),
            PaddingBitsBox::BOX_TYPE,
        ),
        (
            priorities.and_then(|mut table| table.next()).is_some(),
            DegradationPriorityBox::BOX_TYPE,
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

/// Refuses `movie` where a table of the sample table of a track declares samples
pub(super) fn refuse_samples_in_tables(movie: &MovieBox) -> Result<(), MuxError> {
    for trak in &movie.trak {
        let stbl = &trak.mdia.minf.stbl;
        let declared = [
            (
                stbl.stts
                    .iter()
                    .flat_map(|stts| &stts.entries)
                    .any(|entry| entry.sample_count > 0),
                TimeToSampleBox::BOX_TYPE,
            ),
            (
                stbl.ctts
                    .iter()
                    .flat_map(|ctts| &ctts.entries)
                    .any(|entry| entry.sample_count > 0),
                CompositionOffsetBox::BOX_TYPE,
            ),
            (
                stbl.stsc.iter().any(|stsc| !stsc.entries.is_empty()),
                SampleToChunkBox::BOX_TYPE,
            ),
            (sample_count(trak) > 0, SampleSizeBox::BOX_TYPE),
            (
                match &stbl.chunk_offset {
                    Some(ChunkOffsetVariant::ChunkOffset(stco)) => !stco.chunk_offsets.is_empty(),
                    Some(ChunkOffsetVariant::ChunkLargeOffset(co64)) => {
                        !co64.chunk_offsets.is_empty()
                    }
                    Some(_) | None => false,
                },
                ChunkOffsetBox::BOX_TYPE,
            ),
            (
                stbl.stss.iter().any(|stss| !stss.sample_numbers.is_empty()),
                SyncSampleBox::BOX_TYPE,
            ),
            (
                stbl.sdtp.iter().any(|sdtp| !sdtp.entries.is_empty()),
                SampleDependencyTypeBox::BOX_TYPE,
            ),
            (
                stbl.stsh.iter().any(|stsh| !stsh.entries.is_empty()),
                ShadowSyncSampleBox::BOX_TYPE,
            ),
            (
                stbl.padb.iter().any(|padb| !padb.pads.is_empty()),
                PaddingBitsBox::BOX_TYPE,
            ),
            (
                stbl.stdp.iter().any(|stdp| !stdp.priorities.is_empty()),
                DegradationPriorityBox::BOX_TYPE,
            ),
            (
                stbl.sbgp
                    .iter()
                    .flat_map(|sbgp| &sbgp.entries)
                    .any(|entry| entry.sample_count > 0),
                SampleToGroupBox::BOX_TYPE,
            ),
            (
                stbl.csgp
                    .iter()
                    .flat_map(|csgp| &csgp.patterns)
                    .any(|pattern| pattern.sample_count > 0),
                CompactSampleToGroupBox::BOX_TYPE,
            ),
            (
                stbl.subs.iter().any(|subs| !subs.entries.is_empty()),
                SubSampleInformationBox::BOX_TYPE,
            ),
            (
                stbl.ssrt.iter().any(|ssrt| !ssrt.entries.is_empty()),
                SubSampleReferenceTableBox::BOX_TYPE,
            ),
            (
                stbl.saiz.iter().any(|saiz| match &saiz.sample_info_sizes {
                    SampleInfoSizes::Constant { sample_count, .. } => *sample_count > 0,
                    SampleInfoSizes::Table { sample_info_sizes } => !sample_info_sizes.is_empty(),
                    _ => true,
                }),
                SampleAuxiliaryInformationSizesBox::BOX_TYPE,
            ),
            (
                stbl.saio.iter().any(|saio| !saio.offsets.is_empty()),
                SampleAuxiliaryInformationOffsetsBox::BOX_TYPE,
            ),
        ];
        if let Some((_, box_type)) = declared.into_iter().find(|(declared, _)| *declared) {
            return Err(MuxError::SampleTableNotEmpty {
                track_id: trak.tkhd.track_id,
                box_type,
            });
        }
    }

    Ok(())
}

/// What the sample table of a track declares, as the samples written to it so far
#[derive(Debug, Default)]
pub(super) struct TrackTable {
    /// Decoding time past the last sample
    decoding_time: u64,
    /// Chunks, in chunk order
    chunks: Vec<Chunk>,
    /// Samples, in sample order
    samples: Vec<TableSample>,
}

/// What the sample table declares of a chunk
#[derive(Debug)]
struct Chunk {
    /// File offset of the first byte of the chunk
    offset: u64,
    samples_per_chunk: u32,
    sample_description_index: u32,
}

/// What the sample table declares of a sample
#[derive(Debug)]
struct TableSample {
    sample_duration: u32,
    sample_size: u32,
    sample_composition_time_offset: i64,
    sample_flags: SampleFlags,
}

impl TrackTable {
    /// Returns the decoding time past the last sample
    pub(super) const fn decoding_time(&self) -> u64 {
        self.decoding_time
    }

    /// Adds `samples`, described by sample entry `sample_description_index`, as the chunk at file offset `offset`
    pub(super) fn push_chunk(
        &mut self,
        offset: u64,
        sample_description_index: u32,
        samples: &[Sample],
    ) {
        self.chunks.push(Chunk {
            offset,
            samples_per_chunk: u32::try_from(samples.len()).unwrap_or(u32::MAX),
            sample_description_index,
        });
        for sample in samples {
            let properties = sample.properties();
            self.decoding_time = self
                .decoding_time
                .saturating_add(u64::from(properties.sample_duration));
            self.samples.push(TableSample {
                sample_duration: properties.sample_duration,
                sample_size: u32::try_from(sample.data().len()).unwrap_or(u32::MAX),
                sample_composition_time_offset: properties.sample_composition_time_offset,
                sample_flags: properties.sample_flags,
            });
        }
    }

    /// Fills the tables of `stbl` with what the table declares
    ///
    /// The `ctts` is left out where every composition time offset is zero, the
    /// `stss` where every sample is a sync sample, the `sdtp` where no sample
    /// states a dependency, the `padb` where no sample is padded, and the `stdp`
    /// where every degradation priority is zero (§8.6.1.3, §8.6.2, §8.6.4,
    /// §8.7.6, §8.5.3).
    pub(super) fn fill(self, stbl: &mut SampleTableBox) {
        let mut stts = TimeToSampleBox::default();
        stts.entries = runs_of(self.samples.iter().map(|sample| sample.sample_duration))
            .map(|(sample_count, sample_delta)| {
                let mut entry = TimeToSampleEntry::default();
                entry.sample_count = sample_count;
                entry.sample_delta = sample_delta;
                entry
            })
            .collect();
        stbl.stts = Some(stts);

        stbl.ctts = self
            .samples
            .iter()
            .any(|sample| sample.sample_composition_time_offset != 0)
            .then(|| {
                let mut ctts = CompositionOffsetBox::default();
                ctts.version = u8::from(
                    self.samples
                        .iter()
                        .any(|sample| sample.sample_composition_time_offset < 0),
                );
                ctts.entries = runs_of(
                    self.samples
                        .iter()
                        .map(|sample| sample.sample_composition_time_offset),
                )
                .map(|(sample_count, sample_offset)| {
                    let mut entry = CompositionOffsetEntry::default();
                    entry.sample_count = sample_count;
                    entry.sample_offset = sample_offset;
                    entry
                })
                .collect();
                ctts
            });

        let mut stsc = SampleToChunkBox::default();
        let mut previous = None;
        for (first_chunk, chunk) in (1_u32..).zip(&self.chunks) {
            let samples_per_chunk = chunk.samples_per_chunk;
            let sample_description_index = chunk.sample_description_index;
            if previous != Some((samples_per_chunk, sample_description_index)) {
                let mut entry = SampleToChunkEntry::default();
                entry.first_chunk = first_chunk;
                entry.samples_per_chunk = samples_per_chunk;
                entry.sample_description_index = sample_description_index;
                stsc.entries.push(entry);
                previous = Some((samples_per_chunk, sample_description_index));
            }
        }
        stbl.stsc = Some(stsc);

        let sizes: Vec<u32> = self
            .samples
            .iter()
            .map(|sample| sample.sample_size)
            .collect();
        let mut stsz = SampleSizeBox::default();
        stsz.sample_sizes = match (sizes.first(), u32::try_from(sizes.len())) {
            (Some(&size), Ok(sample_count)) if sizes.iter().all(|&other| other == size) => {
                match NonZeroU32::new(size) {
                    Some(sample_size) => SampleSizes::Constant {
                        sample_size,
                        sample_count,
                    },
                    None => SampleSizes::Table { entry_sizes: sizes },
                }
            }
            _ => SampleSizes::Table { entry_sizes: sizes },
        };
        stbl.sample_size = Some(SampleSizeVariant::SampleSize(stsz));

        stbl.chunk_offset = Some(
            match self
                .chunks
                .iter()
                .map(|chunk| u32::try_from(chunk.offset).ok())
                .collect::<Option<Vec<u32>>>()
            {
                Some(chunk_offsets) => {
                    let mut stco = ChunkOffsetBox::default();
                    stco.chunk_offsets = chunk_offsets;
                    ChunkOffsetVariant::ChunkOffset(stco)
                }
                None => {
                    let mut co64 = ChunkLargeOffsetBox::default();
                    co64.chunk_offsets = self.chunks.iter().map(|chunk| chunk.offset).collect();
                    ChunkOffsetVariant::ChunkLargeOffset(co64)
                }
            },
        );

        stbl.stss = self
            .samples
            .iter()
            .any(|sample| sample.sample_flags.sample_is_non_sync_sample)
            .then(|| {
                let mut stss = SyncSampleBox::default();
                stss.sample_numbers = (1_u32..)
                    .zip(&self.samples)
                    .filter(|(_, sample)| !sample.sample_flags.sample_is_non_sync_sample)
                    .map(|(sample_number, _)| sample_number)
                    .collect();
                stss
            });

        let dependency = |flags: &SampleFlags| {
            let mut entry = SampleDependencyTypeEntry::default();
            entry.is_leading = flags.is_leading;
            entry.sample_depends_on = flags.sample_depends_on;
            entry.sample_is_depended_on = flags.sample_is_depended_on;
            entry.sample_has_redundancy = flags.sample_has_redundancy;
            entry
        };
        stbl.sdtp = self
            .samples
            .iter()
            .any(|sample| dependency(&sample.sample_flags) != SampleDependencyTypeEntry::default())
            .then(|| {
                let mut sdtp = SampleDependencyTypeBox::default();
                sdtp.entries = self
                    .samples
                    .iter()
                    .map(|sample| dependency(&sample.sample_flags))
                    .collect();
                sdtp
            });

        stbl.padb = self
            .samples
            .iter()
            .any(|sample| sample.sample_flags.sample_padding_value != 0)
            .then(|| {
                let mut padb = PaddingBitsBox::default();
                padb.pads = self
                    .samples
                    .iter()
                    .map(|sample| sample.sample_flags.sample_padding_value)
                    .collect();
                padb
            });

        stbl.stdp = self
            .samples
            .iter()
            .any(|sample| sample.sample_flags.sample_degradation_priority != 0)
            .then(|| {
                let mut stdp = DegradationPriorityBox::default();
                stdp.priorities = self
                    .samples
                    .iter()
                    .map(|sample| sample.sample_flags.sample_degradation_priority)
                    .collect();
                stdp
            });
    }
}

/// Returns the runs of equal values in `values`: the number of each run, and its value
fn runs_of<Value: PartialEq>(
    values: impl Iterator<Item = Value>,
) -> impl Iterator<Item = (u32, Value)> {
    let mut runs: Vec<(u32, Value)> = Vec::new();
    for value in values {
        match runs.last_mut() {
            Some((count, last)) if *last == value && *count < u32::MAX => {
                *count = count.saturating_add(1)
            }
            Some(_) | None => runs.push((1, value)),
        }
    }
    runs.into_iter()
}

/// Returns the value of each sample a table of runs of samples of one value states, in sample order
pub(super) fn expand<Value: Clone>(
    runs: impl Iterator<Item = (u32, Value)>,
) -> impl Iterator<Item = Value> {
    runs.flat_map(|(sample_count, value)| {
        iter::repeat_n(value, usize::try_from(sample_count).unwrap_or(usize::MAX))
    })
}

/// Movies built from the fields of their boxes, which the tests of resolving share
#[cfg(test)]
pub(crate) mod fixtures {
    use alloc::vec::Vec;

    use isobmff_core::{BoxDefinition, BoxEncode};

    use crate::structure_data::StructureDataLimits;

    use isobmff_boxes::{
        ChunkLargeOffsetBox, ChunkOffsetBox, ChunkOffsetVariant, DataEntry, DataEntryUrlBox,
        MovieBox, SampleDescription, SampleEntry, SampleSizeBox, SampleSizeVariant, SampleSizes,
        SampleToChunkBox, SampleToChunkEntry, TimeToSampleBox, TimeToSampleEntry, TrackBox,
    };

    /// Returns the track `track_id` of the samples of `sizes`, each lasting `sample_delta`, laid in chunks at `chunk_offsets` by the runs `(first_chunk, samples_per_chunk)`
    ///
    /// The track has one sample entry, whose media data is in the file itself.
    pub(crate) fn track(
        track_id: u32,
        sample_delta: u32,
        sizes: &[u32],
        runs: &[(u32, u32)],
        chunk_offsets: &[u64],
    ) -> TrackBox {
        let mut trak = TrackBox::default();
        trak.tkhd.track_id = track_id;
        let stbl = &mut trak.mdia.minf.stbl;
        let mut entry = SampleEntry::default();
        entry.data_reference_index = 1;
        stbl.stsd.sample_descriptions = Vec::from([SampleDescription::Typed(entry)]);
        let mut stts = TimeToSampleBox::default();
        let mut run = TimeToSampleEntry::default();
        run.sample_count = u32::try_from(sizes.len()).unwrap();
        run.sample_delta = sample_delta;
        stts.entries = Vec::from([run]);
        stbl.stts = Some(stts);
        let mut stsc = SampleToChunkBox::default();
        stsc.entries = runs
            .iter()
            .map(|&(first_chunk, samples_per_chunk)| {
                let mut entry = SampleToChunkEntry::default();
                entry.first_chunk = first_chunk;
                entry.samples_per_chunk = samples_per_chunk;
                entry.sample_description_index = 1;
                entry
            })
            .collect();
        stbl.stsc = Some(stsc);
        let mut stsz = SampleSizeBox::default();
        stsz.sample_sizes = SampleSizes::Table {
            entry_sizes: sizes.to_vec(),
        };
        stbl.sample_size = Some(SampleSizeVariant::SampleSize(stsz));
        stbl.chunk_offset = Some(
            match chunk_offsets
                .iter()
                .map(|&offset| u32::try_from(offset))
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(compact) => {
                    let mut stco = ChunkOffsetBox::default();
                    stco.chunk_offsets = compact;
                    ChunkOffsetVariant::ChunkOffset(stco)
                }
                Err(_) => {
                    let mut co64 = ChunkLargeOffsetBox::default();
                    co64.chunk_offsets = chunk_offsets.to_vec();
                    ChunkOffsetVariant::ChunkLargeOffset(co64)
                }
            },
        );
        trak.mdia.minf.dinf.dref.data_entries =
            Vec::from([DataEntry::Url(DataEntryUrlBox::default())]);
        trak
    }

    /// Returns `value` written as a box
    pub(crate) fn encoded(value: &(impl BoxEncode + BoxDefinition)) -> Vec<u8> {
        let mut bytes = alloc::vec![0; usize::try_from(value.encoded_len()).unwrap()];
        value.encode(&mut bytes).unwrap();
        bytes
    }

    /// Returns the box of `box_type` whose payload is `fields` laid end to end, under a 32-bit size
    pub(crate) fn written(box_type: &[u8; 4], fields: &[&[u8]]) -> Vec<u8> {
        let payload = fields.concat();
        let size = u32::try_from(payload.len().checked_add(8).unwrap()).unwrap();
        [&size.to_be_bytes()[..], box_type, &payload].concat()
    }

    /// Limits no structure-data of the tests reaches
    pub(crate) const UNLIMITED: StructureDataLimits = StructureDataLimits {
        sample_count: u64::MAX,
        segment_index_count: u64::MAX,
        subsegment_count: u64::MAX,
    };

    /// Returns the movie of `tracks`
    pub(crate) fn movie(tracks: impl IntoIterator<Item = TrackBox>) -> MovieBox {
        let mut movie = MovieBox::default();
        movie.trak = tracks.into_iter().collect();
        movie
    }
}
