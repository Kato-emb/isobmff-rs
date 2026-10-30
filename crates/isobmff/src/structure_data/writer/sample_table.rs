//! The sample table of a track, filled with the samples written to it, ISO/IEC 14496-12 §8.5 and §8.7

use alloc::vec::Vec;
use core::num::NonZeroU32;

use isobmff_boxes::{
    ChunkLargeOffsetBox, ChunkOffsetBox, ChunkOffsetVariant, CompositionOffsetBox,
    CompositionOffsetEntry, SampleDependencyTypeBox, SampleDependencyTypeEntry, SampleFlags,
    SampleSizeBox, SampleSizeVariant, SampleSizes, SampleTableBox, SampleToChunkBox,
    SampleToChunkEntry, SyncSampleBox, TimeToSampleBox, TimeToSampleEntry,
};

use crate::sample::Sample;

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
    /// `stss` where every sample is a sync sample, and the `sdtp` where no sample
    /// states a dependency (§8.6.1.3, §8.6.2, §8.6.4).
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
