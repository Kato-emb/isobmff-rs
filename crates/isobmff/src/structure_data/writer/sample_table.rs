//! The sample table of a track, filled with the samples written to it, ISO/IEC 14496-12 §8.5 and §8.7

use alloc::vec::Vec;
use core::num::NonZeroU32;

use isobmff_boxes::{
    ChunkLargeOffsetBox, ChunkOffsetBox, ChunkOffsetVariant, CompositionOffsetBox,
    CompositionOffsetEntry, SampleDependencyTypeBox, SampleDependencyTypeEntry, SampleFlags,
    SampleSizeBox, SampleSizeVariant, SampleSizes, SampleTableBox, SampleToChunkBox,
    SampleToChunkEntry, SyncSampleBox, TimeToSampleBox, TimeToSampleEntry,
};

/// What the sample table of a track declares, as the samples written to it so far
#[derive(Debug, Default)]
pub(super) struct TrackTable {
    /// Decoding time past the last sample
    pub(super) decoding_time: u64,
    /// Number of samples and sample entry of each chunk, in chunk order
    pub(super) chunks: Vec<(u32, u32)>,
    /// File offset of each chunk, in chunk order
    pub(super) chunk_offsets: Vec<u64>,
    /// Duration, size, composition time offset and flags of each sample, in sample order
    pub(super) samples: Vec<(u32, u32, i64, SampleFlags)>,
}

impl TrackTable {
    /// Fills the tables of `stbl` with what the table declares
    ///
    /// The `ctts` is left out where every composition time offset is zero, the
    /// `stss` where every sample is a sync sample, and the `sdtp` where no sample
    /// states a dependency (§8.6.1.3, §8.6.2, §8.6.4).
    pub(super) fn fill(self, stbl: &mut SampleTableBox) {
        let mut stts = TimeToSampleBox::default();
        stts.entries = runs_of(self.samples.iter().map(|sample| sample.0))
            .map(|(sample_count, sample_delta)| {
                let mut entry = TimeToSampleEntry::default();
                entry.sample_count = sample_count;
                entry.sample_delta = sample_delta;
                entry
            })
            .collect();
        stbl.stts = Some(stts);

        stbl.ctts = self.samples.iter().any(|sample| sample.2 != 0).then(|| {
            let mut ctts = CompositionOffsetBox::default();
            ctts.version = u8::from(self.samples.iter().any(|sample| sample.2 < 0));
            ctts.entries = runs_of(self.samples.iter().map(|sample| sample.2))
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
        for (first_chunk, &(samples_per_chunk, sample_description_index)) in
            (1_u32..).zip(&self.chunks)
        {
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

        let sizes: Vec<u32> = self.samples.iter().map(|sample| sample.1).collect();
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
                .chunk_offsets
                .iter()
                .map(|&offset| u32::try_from(offset).ok())
                .collect::<Option<Vec<u32>>>()
            {
                Some(chunk_offsets) => {
                    let mut stco = ChunkOffsetBox::default();
                    stco.chunk_offsets = chunk_offsets;
                    ChunkOffsetVariant::ChunkOffset(stco)
                }
                None => {
                    let mut co64 = ChunkLargeOffsetBox::default();
                    co64.chunk_offsets = self.chunk_offsets;
                    ChunkOffsetVariant::ChunkLargeOffset(co64)
                }
            },
        );

        stbl.stss = self
            .samples
            .iter()
            .any(|sample| sample.3.sample_is_non_sync_sample)
            .then(|| {
                let mut stss = SyncSampleBox::default();
                stss.sample_numbers = (1_u32..)
                    .zip(&self.samples)
                    .filter(|(_, sample)| !sample.3.sample_is_non_sync_sample)
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
            .any(|sample| dependency(&sample.3) != SampleDependencyTypeEntry::default())
            .then(|| {
                let mut sdtp = SampleDependencyTypeBox::default();
                sdtp.entries = self
                    .samples
                    .iter()
                    .map(|sample| dependency(&sample.3))
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
