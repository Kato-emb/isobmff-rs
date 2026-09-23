//! [`OpenTrack`], what the samples of one track state, table by table, until the samples are over

use alloc::vec::Vec;

use isobmff_boxes::{
    ChunkOffsets, CompositionOffsetBox, CompositionTimeOffset, DegradationPriorityBox,
    PaddingBitsBox, SampleDependencyTypeBox, SampleFlags, SampleSizeBox, SampleSizes,
    SampleToChunkBox, SyncSampleBox, SyncSampleEntry, TimeToSampleBox,
};

use crate::error::Error;
use crate::sample::Sample;
use crate::sample_table_writer::SampleTables;

/// What the samples of one track handed over so far state, table by table
///
/// `reached` is where those samples leave the decode timeline of the track,
/// and `chunks` holds `(sample_count, sample_description_index)` per chunk
/// closed, as [`SampleToChunkBox::from_chunks`] takes them.
#[derive(Clone, Debug, Default)]
pub(super) struct OpenTrack {
    pub(super) reached: u64,
    pub(super) deltas: Vec<u32>,
    pub(super) sizes: Vec<u32>,
    pub(super) offsets: Vec<CompositionTimeOffset>,
    pub(super) sample_flags: Vec<SampleFlags>,
    pub(super) chunks: Vec<(u64, u32)>,
    pub(super) chunk_offsets: Vec<u64>,
}

impl OpenTrack {
    /// Places `sample` on the tables of this track, and hands its bytes back
    pub(super) fn place(&mut self, sample: Sample) -> Result<Vec<u8>, Error> {
        let track_id = sample.track_id();
        let offset = sample.sample_composition_time_offset();
        let Some(sample_composition_time_offset) = CompositionTimeOffset::new(offset) else {
            return Err(Error::composition_time_offset_out_of_range(
                track_id, offset,
            ));
        };
        let offered = sample.data().len() as u64;
        let Ok(sample_size) = u32::try_from(offered) else {
            return Err(Error::sample_size_out_of_range(track_id, offered));
        };
        if sample.decode_time() != self.reached {
            return Err(Error::decode_time_mismatch(
                track_id,
                sample.decode_time(),
                self.reached,
            ));
        }
        self.reached = self
            .reached
            .checked_add(u64::from(sample.sample_duration()))
            .ok_or(Error::decode_time_overflow(track_id))?;
        self.deltas.push(sample.sample_duration());
        self.sizes.push(sample_size);
        self.offsets.push(sample_composition_time_offset);
        self.sample_flags.push(sample.sample_flags());

        Ok(sample.into_data())
    }

    /// Builds the tables of track `track_id`, now that its samples are over
    pub(super) fn into_tables(self, track_id: u32) -> Result<SampleTables, Error> {
        let ctts = if self.offsets.iter().all(|offset| offset.get() == 0) {
            None
        } else {
            let widest = || self.offsets.iter().map(|offset| offset.get()).max();
            let ctts = CompositionOffsetBox::from_offsets(self.offsets.iter().copied())
                .ok_or_else(|| {
                    Error::composition_time_offset_out_of_range(
                        track_id,
                        widest().unwrap_or_default(),
                    )
                })?;
            Some(ctts)
        };
        let flags = &self.sample_flags;
        let stss = flags
            .iter()
            .any(|sample| sample.sample_is_non_sync_sample())
            .then(|| {
                SyncSampleBox::new(
                    (1..)
                        .zip(flags)
                        .filter(|(_, sample)| !sample.sample_is_non_sync_sample())
                        .map(|(sample_number, _)| SyncSampleEntry::new(sample_number))
                        .collect(),
                )
            });
        let sdtp = stated(
            flags
                .iter()
                .map(|sample| sample.sample_dependency_type())
                .collect(),
        )
        .map(SampleDependencyTypeBox::new);
        let padb = stated(flags.iter().map(|sample| sample.padding_bits()).collect())
            .map(PaddingBitsBox::new);
        let stdp = stated(
            flags
                .iter()
                .map(|sample| sample.degradation_priority())
                .collect(),
        )
        .map(DegradationPriorityBox::new);

        Ok(SampleTables {
            stts: TimeToSampleBox::from_deltas(self.deltas),
            stsc: SampleToChunkBox::from_chunks(self.chunks)?,
            sample_sizes: SampleSizes::Stsz(SampleSizeBox::from_sizes(self.sizes)),
            chunk_offsets: ChunkOffsets::from_offsets(self.chunk_offsets),
            ctts,
            stss,
            sdtp,
            padb,
            stdp,
        })
    }
}

/// Returns the entries of a table, unless every one states what the absence of the table does
fn stated<Entry: Default + PartialEq>(entries: Vec<Entry>) -> Option<Vec<Entry>> {
    entries
        .iter()
        .any(|entry| *entry != Entry::default())
        .then_some(entries)
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeMap;
    use alloc::vec;

    use isobmff_boxes::{
        ChunkOffsetBox, ChunkOffsetEntry, ChunkOffsets, CompositionOffsetBox,
        CompositionTimeOffset, DegradationPriorityBox, DegradationPriorityEntry, PaddingBitsBox,
        PaddingBitsEntry, SampleDependencyTypeBox, SampleDependencyTypeEntry, SampleFlags,
        SampleSizeBox, SampleSizes, SampleToChunkBox, SampleToChunkEntry, SyncSampleBox,
        SyncSampleEntry, TimeToSampleBox,
    };

    use crate::error::Error;
    use crate::sample::Sample;
    use crate::sample_table_writer::tests::{laid_out, sample};
    use crate::sample_table_writer::{SampleTableWriter, SampleTables};

    use super::OpenTrack;

    #[test]
    fn a_track_starting_anywhere_but_at_zero_is_refused() {
        let mut writer = SampleTableWriter::new();

        writer.begin_chunk(1_000).unwrap();

        assert_eq!(
            writer.handle_sample(sample(1, 512, b"AAAA")),
            Err(Error::decode_time_mismatch(1, 512, 0))
        );
    }

    #[test]
    fn a_sample_that_does_not_start_where_the_one_before_it_ends_is_refused() {
        let mut writer = SampleTableWriter::new();

        writer.begin_chunk(1_000).unwrap();
        writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();
        writer.begin_chunk(2_000).unwrap();

        assert_eq!(
            writer.handle_sample(sample(1, 512, b"BBBB")),
            Err(Error::decode_time_mismatch(1, 512, 1_024))
        );
    }

    #[test]
    fn decode_times_running_past_what_64_bits_carry_are_refused() {
        let at_the_end_of_time =
            Sample::new(1, u64::MAX, 1, 0, SampleFlags::ZERO, 1, b"AAAA".to_vec());
        let mut track = OpenTrack {
            reached: u64::MAX,
            ..OpenTrack::default()
        };

        assert_eq!(
            track.place(at_the_end_of_time),
            Err(Error::decode_time_overflow(1))
        );
    }

    #[test]
    fn a_sample_composed_further_off_than_a_ctts_writes_is_refused() {
        let too_early = Sample::new(
            1,
            0,
            1_024,
            -(1 << 31) - 1,
            SampleFlags::ZERO,
            1,
            b"AAAA".to_vec(),
        );
        let mut writer = SampleTableWriter::new();

        writer.begin_chunk(1_000).unwrap();

        assert_eq!(
            writer.handle_sample(too_early),
            Err(Error::composition_time_offset_out_of_range(
                1,
                -(1 << 31) - 1
            ))
        );
    }

    /// Sample of track 1 at `decode_time` lasting 1024 units, stating `sample_composition_time_offset` and `sample_flags`
    fn stating(
        decode_time: u64,
        sample_composition_time_offset: i64,
        sample_flags: SampleFlags,
    ) -> Sample {
        Sample::new(
            1,
            decode_time,
            1_024,
            sample_composition_time_offset,
            sample_flags,
            1,
            b"AAAA".to_vec(),
        )
    }

    /// Flags of a sample that states nothing but being left out of the sync samples
    fn non_sync() -> SampleFlags {
        SampleFlags::new(
            SampleDependencyTypeEntry::default(),
            PaddingBitsEntry::default(),
            true,
            DegradationPriorityEntry::default(),
        )
    }

    #[test]
    fn a_track_gets_the_optional_tables_its_samples_call_for() {
        let tables = laid_out(vec![(
            1_000,
            vec![
                stating(
                    0,
                    8,
                    SampleFlags::new(
                        SampleDependencyTypeEntry::new(2, 2, 1, 2).unwrap(),
                        PaddingBitsEntry::new(5).unwrap(),
                        false,
                        DegradationPriorityEntry::new(3),
                    ),
                ),
                stating(
                    1_024,
                    -2,
                    SampleFlags::new(
                        SampleDependencyTypeEntry::new(0, 1, 0, 0).unwrap(),
                        PaddingBitsEntry::default(),
                        true,
                        DegradationPriorityEntry::default(),
                    ),
                ),
                stating(2_048, 0, SampleFlags::ZERO),
            ],
        )]);
        let offset = |value| CompositionTimeOffset::new(value).unwrap();

        assert_eq!(
            tables,
            BTreeMap::from([(
                1,
                SampleTables {
                    stts: TimeToSampleBox::from_deltas([1_024; 3]),
                    stsc: SampleToChunkBox::new(vec![SampleToChunkEntry::new(1, 3, 1)]),
                    sample_sizes: SampleSizes::Stsz(SampleSizeBox::from_sizes([4; 3])),
                    chunk_offsets: ChunkOffsets::Stco(ChunkOffsetBox::new(vec![
                        ChunkOffsetEntry::new(1_000)
                    ])),
                    ctts: CompositionOffsetBox::from_offsets([offset(8), offset(-2), offset(0)]),
                    stss: Some(SyncSampleBox::new(vec![
                        SyncSampleEntry::new(1),
                        SyncSampleEntry::new(3),
                    ])),
                    sdtp: Some(SampleDependencyTypeBox::new(vec![
                        SampleDependencyTypeEntry::new(2, 2, 1, 2).unwrap(),
                        SampleDependencyTypeEntry::new(0, 1, 0, 0).unwrap(),
                        SampleDependencyTypeEntry::default(),
                    ])),
                    padb: Some(PaddingBitsBox::new(vec![
                        PaddingBitsEntry::new(5).unwrap(),
                        PaddingBitsEntry::default(),
                        PaddingBitsEntry::default(),
                    ])),
                    stdp: Some(DegradationPriorityBox::new(vec![
                        DegradationPriorityEntry::new(3),
                        DegradationPriorityEntry::default(),
                        DegradationPriorityEntry::default(),
                    ])),
                },
            )])
        );
    }

    #[test]
    fn a_track_of_no_sync_sample_lists_none_in_its_stss() {
        let tables = laid_out(vec![(
            1_000,
            vec![stating(0, 0, non_sync()), stating(1_024, 0, non_sync())],
        )]);

        assert_eq!(
            tables.get(&1).and_then(SampleTables::stss),
            Some(&SyncSampleBox::new(vec![]))
        );
    }

    #[test]
    fn a_track_composing_samples_both_early_and_past_what_signed_offsets_reach_is_refused() {
        let mut writer = SampleTableWriter::new();

        writer.begin_chunk(1_000).unwrap();
        writer
            .handle_sample(Sample::new(
                1,
                0,
                1_024,
                -8,
                SampleFlags::ZERO,
                1,
                b"AAAA".to_vec(),
            ))
            .unwrap();
        writer
            .handle_sample(Sample::new(
                1,
                1_024,
                1_024,
                1 << 31,
                SampleFlags::ZERO,
                1,
                b"BBBB".to_vec(),
            ))
            .unwrap();

        assert_eq!(
            writer.finish(),
            Err(Error::composition_time_offset_out_of_range(1, 1 << 31))
        );
    }
}
