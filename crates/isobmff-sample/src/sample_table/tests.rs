use alloc::vec;
use alloc::vec::Vec;
use core::num::NonZeroU32;
use core::ops::Range;

use isobmff_boxes::{
    ChunkLargeOffsetBox, ChunkLargeOffsetEntry, ChunkOffsetBox, ChunkOffsetEntry, ChunkOffsets,
    CompactSampleSizeBox, CompositionOffsetBox, CompositionTimeOffset, DegradationPriorityBox,
    DegradationPriorityEntry, HeaderDuration, MovieBox, MovieHeaderBox, PaddingBitsBox,
    PaddingBitsEntry, SampleDependencyTypeBox, SampleDependencyTypeEntry, SampleDescriptionBox,
    SampleFlags, SampleSizeBox, SampleSizeEntries, SampleSizeEntry, SampleSizes, SampleTableBox,
    SampleToChunkBox, SampleToChunkEntry, SyncSampleBox, SyncSampleEntry, TimeToSampleBox,
    TimeToSampleEntry, TrackBox,
};
use isobmff_core::{AnyBox, BoxType, Mp4EpochSeconds};
use isobmff_test_support::{
    external_data_reference, sample_table, self_contained_data_reference, track_laid_out,
    unfragmented_movie,
};

use super::sample_extents;
use crate::error::Error;
use crate::sample::SampleExtent;

/// Movie of the given tracks, continued in no fragment
fn movie(trak: Vec<TrackBox>) -> MovieBox {
    MovieBox::new(
        MovieHeaderBox::new(
            Mp4EpochSeconds::from_seconds(0),
            Mp4EpochSeconds::from_seconds(0),
            1_000,
            HeaderDuration::ZERO,
            2,
        ),
        trak,
        None,
    )
    .unwrap()
}

/// Decode timeline of runs of samples, each `(sample_count, sample_delta)`
fn stts(entries: &[(u32, u32)]) -> TimeToSampleBox {
    TimeToSampleBox::new(
        entries
            .iter()
            .map(|&(sample_count, sample_delta)| TimeToSampleEntry::new(sample_count, sample_delta))
            .collect(),
    )
}

/// Runs of chunks, each `(first_chunk, samples_per_chunk)`, described by the one entry
fn stsc(entries: &[(u32, u32)]) -> SampleToChunkBox {
    SampleToChunkBox::new(
        entries
            .iter()
            .map(|&(first_chunk, samples_per_chunk)| {
                SampleToChunkEntry::new(first_chunk, samples_per_chunk, 1)
            })
            .collect(),
    )
}

/// Sizes stated one per sample
fn stsz(sizes: &[u32]) -> SampleSizes {
    SampleSizes::Stsz(SampleSizeBox::new(SampleSizeEntries::PerSample(
        sizes.iter().copied().map(SampleSizeEntry::new).collect(),
    )))
}

/// Chunks starting at the offsets given, stated in 32 bits
fn stco(offsets: &[u32]) -> ChunkOffsets {
    ChunkOffsets::Stco(ChunkOffsetBox::new(
        offsets.iter().copied().map(ChunkOffsetEntry::new).collect(),
    ))
}

/// Track `track_id` of the file itself, its samples laid out by the four tables
fn track_of(
    track_id: u32,
    stts: TimeToSampleBox,
    stsc: SampleToChunkBox,
    sample_sizes: SampleSizes,
    chunk_offsets: ChunkOffsets,
) -> TrackBox {
    track_laid_out(
        track_id,
        self_contained_data_reference(),
        sample_table(stts, stsc, sample_sizes, chunk_offsets),
    )
}

/// Track 1 of four-byte samples lasting 100 units, one to a chunk, at the offsets given
fn track_chunked_at(offsets: &[u32]) -> TrackBox {
    track_of(
        1,
        stts(&[(u32::try_from(offsets.len()).unwrap(), 100)]),
        stsc(&[(1, 1)]),
        stsz(&vec![4; offsets.len()]),
        stco(offsets),
    )
}

/// Track 1 of three four-byte samples lasting 100 units in a chunk at 100, its sample table put through `stating`
fn three_samples_stating(stating: impl FnOnce(SampleTableBox) -> SampleTableBox) -> TrackBox {
    track_laid_out(
        1,
        self_contained_data_reference(),
        stating(sample_table(
            stts(&[(3, 100)]),
            stsc(&[(1, 3)]),
            stsz(&[4; 3]),
            stco(&[100]),
        )),
    )
}

/// Composition time offsets stated one per sample
fn ctts(offsets: &[i64]) -> CompositionOffsetBox {
    CompositionOffsetBox::from_offsets(
        offsets
            .iter()
            .map(|&offset| CompositionTimeOffset::new(offset).unwrap()),
    )
    .unwrap()
}

/// Sync samples listed by their numbers
fn stss(sample_numbers: &[u32]) -> SyncSampleBox {
    SyncSampleBox::new(
        sample_numbers
            .iter()
            .copied()
            .map(SyncSampleEntry::new)
            .collect(),
    )
}

/// Extent of a sync sample of `track_id` described by entry 1, in the file itself
fn extent(track_id: u32, decode_time: u64, sample_duration: u32, data: Range<u64>) -> SampleExtent {
    SampleExtent::new(
        track_id,
        decode_time,
        sample_duration,
        0,
        SampleFlags::ZERO,
        1,
        1,
        data,
    )
}

/// Resolves the samples of `movie`, whole
fn resolved(movie: &MovieBox) -> Result<Vec<SampleExtent>, Error> {
    sample_extents(movie).collect()
}

#[test]
fn a_sample_is_read_across_the_four_tables_of_its_track() {
    let trak = track_of(
        1,
        stts(&[(2, 100), (3, 50)]),
        stsc(&[(1, 2), (2, 3)]),
        stsz(&[4, 6, 2, 8, 3]),
        stco(&[1_000, 2_000]),
    );

    assert_eq!(
        resolved(&movie(vec![trak])),
        Ok(vec![
            extent(1, 0, 100, 1_000..1_004),
            extent(1, 100, 100, 1_004..1_010),
            extent(1, 200, 50, 2_000..2_002),
            extent(1, 250, 50, 2_002..2_010),
            extent(1, 300, 50, 2_010..2_013),
        ])
    );
}

#[test]
fn a_run_of_chunks_reaches_the_chunk_the_next_run_starts_at() {
    let trak = track_of(
        1,
        stts(&[(6, 100)]),
        stsc(&[(1, 1), (3, 2)]),
        stsz(&[4; 6]),
        stco(&[100, 200, 300, 400]),
    );

    assert_eq!(
        resolved(&movie(vec![trak])),
        Ok(vec![
            extent(1, 0, 100, 100..104),
            extent(1, 100, 100, 200..204),
            extent(1, 200, 100, 300..304),
            extent(1, 300, 100, 304..308),
            extent(1, 400, 100, 400..404),
            extent(1, 500, 100, 404..408),
        ])
    );
}

#[test]
fn a_track_stating_its_chunk_offsets_in_64_bits_resolves_as_one_stating_them_in_32() {
    let in_64_bits = track_of(
        1,
        stts(&[(2, 100)]),
        stsc(&[(1, 1)]),
        stsz(&[4, 4]),
        ChunkOffsets::Co64(ChunkLargeOffsetBox::new(vec![
            ChunkLargeOffsetEntry::new(100),
            ChunkLargeOffsetEntry::new(200),
        ])),
    );

    assert_eq!(
        resolved(&movie(vec![in_64_bits])),
        resolved(&movie(vec![track_chunked_at(&[100, 200])]))
    );
}

#[test]
fn a_size_every_sample_shares_is_read_for_each_of_them() {
    let trak = track_of(
        1,
        stts(&[(3, 100)]),
        stsc(&[(1, 3)]),
        SampleSizes::Stsz(SampleSizeBox::new(SampleSizeEntries::Uniform {
            sample_size: NonZeroU32::new(4).unwrap(),
            sample_count: 3,
        })),
        stco(&[100]),
    );

    assert_eq!(
        resolved(&movie(vec![trak])),
        Ok(vec![
            extent(1, 0, 100, 100..104),
            extent(1, 100, 100, 104..108),
            extent(1, 200, 100, 108..112),
        ])
    );
}

#[test]
fn a_track_stating_its_sample_sizes_in_a_stz2_resolves_as_one_stating_them_in_a_stsz() {
    let in_a_stz2 = track_of(
        1,
        stts(&[(3, 100)]),
        stsc(&[(1, 3)]),
        SampleSizes::Stz2(CompactSampleSizeBox::from_sizes([4, 12, 7])),
        stco(&[100]),
    );
    let in_a_stsz = track_of(
        1,
        stts(&[(3, 100)]),
        stsc(&[(1, 3)]),
        stsz(&[4, 12, 7]),
        stco(&[100]),
    );

    assert_eq!(
        resolved(&movie(vec![in_a_stz2])),
        resolved(&movie(vec![in_a_stsz]))
    );
}

#[test]
fn the_samples_of_a_run_take_the_description_it_names() {
    let entry = || AnyBox::from_raw_bytes(BoxType::compact(*b"avc1"), vec![0, 0, 0, 0, 0, 0, 0, 1]);
    let trak = track_laid_out(
        1,
        self_contained_data_reference(),
        SampleTableBox::new(
            SampleDescriptionBox::new(vec![entry(), entry()]),
            stts(&[(2, 100)]),
            SampleToChunkBox::new(vec![
                SampleToChunkEntry::new(1, 1, 2),
                SampleToChunkEntry::new(2, 1, 1),
            ]),
            stsz(&[4, 4]),
            stco(&[100, 200]),
        ),
    );

    assert_eq!(
        resolved(&movie(vec![trak])),
        Ok(vec![
            SampleExtent::new(1, 0, 100, 0, SampleFlags::ZERO, 2, 1, 100..104),
            extent(1, 100, 100, 200..204),
        ])
    );
}

#[test]
fn the_chunks_of_two_tracks_come_out_in_the_order_the_file_lays_them_down() {
    let interleaved = movie(vec![
        track_chunked_at(&[100, 300]),
        track_of(
            2,
            stts(&[(4, 1_000)]),
            stsc(&[(1, 2)]),
            stsz(&[8; 4]),
            stco(&[200, 400]),
        ),
    ]);

    assert_eq!(
        resolved(&interleaved),
        Ok(vec![
            extent(1, 0, 100, 100..104),
            extent(2, 0, 1_000, 200..208),
            extent(2, 1_000, 1_000, 208..216),
            extent(1, 100, 100, 300..304),
            extent(2, 2_000, 1_000, 400..408),
            extent(2, 3_000, 1_000, 408..416),
        ])
    );
}

#[test]
fn the_optional_tables_state_the_offset_and_the_flags_of_each_sample() {
    let dependencies = [
        SampleDependencyTypeEntry::new(2, 2, 1, 2).unwrap(),
        SampleDependencyTypeEntry::new(0, 1, 0, 0).unwrap(),
        SampleDependencyTypeEntry::new(3, 1, 2, 1).unwrap(),
    ];
    let paddings = [
        PaddingBitsEntry::new(5).unwrap(),
        PaddingBitsEntry::new(0).unwrap(),
        PaddingBitsEntry::new(7).unwrap(),
    ];
    let priorities = [
        DegradationPriorityEntry::new(3),
        DegradationPriorityEntry::new(0),
        DegradationPriorityEntry::new(0xffff),
    ];
    let trak = three_samples_stating(|stbl| {
        stbl.with_ctts(ctts(&[8, -2, 0]))
            .with_stss(stss(&[1, 3]))
            .with_sdtp(SampleDependencyTypeBox::new(dependencies.to_vec()))
            .with_padb(PaddingBitsBox::new(paddings.to_vec()))
            .with_stdp(DegradationPriorityBox::new(priorities.to_vec()))
    });
    let [first_dependency, second_dependency, third_dependency] = dependencies;
    let [first_padding, second_padding, third_padding] = paddings;
    let [first_priority, second_priority, third_priority] = priorities;

    assert_eq!(
        resolved(&movie(vec![trak])),
        Ok(vec![
            SampleExtent::new(
                1,
                0,
                100,
                8,
                SampleFlags::new(first_dependency, first_padding, false, first_priority),
                1,
                1,
                100..104
            ),
            SampleExtent::new(
                1,
                100,
                100,
                -2,
                SampleFlags::new(second_dependency, second_padding, true, second_priority),
                1,
                1,
                104..108
            ),
            SampleExtent::new(
                1,
                200,
                100,
                0,
                SampleFlags::new(third_dependency, third_padding, false, third_priority),
                1,
                1,
                108..112
            ),
        ])
    );
}

#[test]
fn a_table_missing_on_its_own_leaves_its_fields_as_a_track_stating_none_has_them() {
    let listing_no_sync_sample = three_samples_stating(|stbl| stbl.with_stss(stss(&[])));
    let composing_the_second_late = three_samples_stating(|stbl| stbl.with_ctts(ctts(&[0, 16, 0])));
    let non_sync = SampleFlags::new(
        SampleDependencyTypeEntry::default(),
        PaddingBitsEntry::default(),
        true,
        DegradationPriorityEntry::default(),
    );

    assert_eq!(
        resolved(&movie(vec![listing_no_sync_sample])),
        Ok(vec![
            SampleExtent::new(1, 0, 100, 0, non_sync, 1, 1, 100..104),
            SampleExtent::new(1, 100, 100, 0, non_sync, 1, 1, 104..108),
            SampleExtent::new(1, 200, 100, 0, non_sync, 1, 1, 108..112),
        ])
    );
    assert_eq!(
        resolved(&movie(vec![composing_the_second_late])),
        Ok(vec![
            extent(1, 0, 100, 100..104),
            SampleExtent::new(1, 100, 100, 16, SampleFlags::ZERO, 1, 1, 104..108),
            extent(1, 200, 100, 108..112),
        ])
    );
}

#[test]
fn an_optional_table_counting_other_than_the_samples_of_its_track_is_refused() {
    let tracks = [
        three_samples_stating(|stbl| stbl.with_ctts(ctts(&[8, 8]))),
        three_samples_stating(|stbl| stbl.with_ctts(ctts(&[8; 4]))),
        three_samples_stating(|stbl| {
            stbl.with_sdtp(SampleDependencyTypeBox::new(vec![
                SampleDependencyTypeEntry::default();
                2
            ]))
        }),
        three_samples_stating(|stbl| {
            stbl.with_padb(PaddingBitsBox::new(vec![PaddingBitsEntry::default(); 4]))
        }),
        three_samples_stating(|stbl| {
            stbl.with_stdp(DegradationPriorityBox::new(vec![
                DegradationPriorityEntry::default();
                2
            ]))
        }),
    ];

    for trak in tracks {
        assert_eq!(
            resolved(&movie(vec![trak])),
            Err(Error::sample_count_mismatch(1))
        );
    }
}

#[test]
fn a_sync_sample_listed_out_of_order_or_past_the_samples_is_refused() {
    let listings = [
        (stss(&[2, 2]), Error::sync_sample_out_of_range(1, 2)),
        (stss(&[3, 1]), Error::sync_sample_out_of_range(1, 1)),
        (stss(&[0]), Error::sync_sample_out_of_range(1, 0)),
        (stss(&[1, 4]), Error::sync_sample_out_of_range(1, 4)),
    ];

    for (listing, refused) in listings {
        let trak = three_samples_stating(|stbl| stbl.with_stss(listing));

        assert_eq!(resolved(&movie(vec![trak])), Err(refused));
    }
}

#[test]
fn a_movie_declaring_no_sample_in_its_tables_resolves_to_none() {
    assert_eq!(resolved(&unfragmented_movie()), Ok(vec![]));
}

#[test]
fn tables_counting_different_numbers_of_samples_are_refused() {
    let more_deltas_than_sizes = track_of(
        1,
        stts(&[(4, 100)]),
        stsc(&[(1, 3)]),
        stsz(&[4; 3]),
        stco(&[100]),
    );
    let more_sizes_than_the_chunks_hold = track_of(
        1,
        stts(&[(4, 100)]),
        stsc(&[(1, 3)]),
        stsz(&[4; 4]),
        stco(&[100]),
    );
    let fewer_sizes_than_the_chunks_hold = track_of(
        1,
        stts(&[(2, 100)]),
        stsc(&[(1, 3)]),
        stsz(&[4; 2]),
        stco(&[100]),
    );

    for trak in [
        more_deltas_than_sizes,
        more_sizes_than_the_chunks_hold,
        fewer_sizes_than_the_chunks_hold,
    ] {
        assert_eq!(
            resolved(&movie(vec![trak])),
            Err(Error::sample_count_mismatch(1))
        );
    }
}

#[test]
fn a_run_starting_past_the_last_chunk_is_refused() {
    let past_the_last_chunk = track_of(
        1,
        stts(&[(2, 100)]),
        stsc(&[(1, 2), (3, 1)]),
        stsz(&[4; 2]),
        stco(&[100]),
    );

    assert_eq!(
        resolved(&movie(vec![past_the_last_chunk])),
        Err(Error::first_chunk_out_of_range(1, 3))
    );
}

#[test]
fn a_run_starting_at_or_before_the_start_of_the_run_before_it_is_refused() {
    let doubling_back = track_of(
        1,
        stts(&[(3, 100)]),
        stsc(&[(1, 1), (3, 1), (2, 1)]),
        stsz(&[4; 3]),
        stco(&[100, 200, 300]),
    );
    let starting_twice = track_of(
        1,
        stts(&[(3, 100)]),
        stsc(&[(1, 1), (3, 1), (3, 1)]),
        stsz(&[4; 3]),
        stco(&[100, 200, 300]),
    );

    assert_eq!(
        resolved(&movie(vec![doubling_back])),
        Err(Error::first_chunk_out_of_range(1, 2))
    );
    assert_eq!(
        resolved(&movie(vec![starting_twice])),
        Err(Error::first_chunk_out_of_range(1, 3))
    );
}

#[test]
fn a_first_run_starting_anywhere_but_at_the_first_chunk_is_refused() {
    let starting_at_the_second_chunk = track_of(
        1,
        stts(&[(1, 100)]),
        stsc(&[(2, 1)]),
        stsz(&[4]),
        stco(&[100, 200]),
    );

    assert_eq!(
        resolved(&movie(vec![starting_at_the_second_chunk])),
        Err(Error::first_chunk_out_of_range(1, 2))
    );
}

#[test]
fn chunks_no_run_lays_over_hold_no_sample() {
    let chunks_without_runs = track_of(1, stts(&[]), stsc(&[]), stsz(&[]), stco(&[100, 200]));

    assert_eq!(resolved(&movie(vec![chunks_without_runs])), Ok(vec![]));
}

#[test]
fn a_run_described_by_an_entry_its_track_has_none_of_is_refused() {
    let trak = track_of(
        1,
        stts(&[(1, 100)]),
        SampleToChunkBox::new(vec![SampleToChunkEntry::new(1, 1, 2)]),
        stsz(&[4]),
        stco(&[100]),
    );

    assert_eq!(
        resolved(&movie(vec![trak])),
        Err(Error::unknown_sample_description_index(1, 2))
    );
}

#[test]
fn a_sample_of_a_track_reading_from_an_external_file_is_refused() {
    let trak = track_laid_out(
        1,
        external_data_reference(),
        sample_table(stts(&[(1, 100)]), stsc(&[(1, 1)]), stsz(&[4]), stco(&[100])),
    );

    assert_eq!(
        resolved(&movie(vec![trak])),
        Err(Error::external_data_reference(1, 1))
    );
}

#[test]
fn the_extents_resolved_before_a_failure_come_out_ahead_of_it() {
    let then_a_track_out_of_range = movie(vec![
        track_chunked_at(&[100, 200]),
        track_of(2, stts(&[]), stsc(&[(2, 1)]), stsz(&[]), stco(&[])),
    ]);

    assert_eq!(
        sample_extents(&then_a_track_out_of_range).collect::<Vec<_>>(),
        [
            Ok(extent(1, 0, 100, 100..104)),
            Ok(extent(1, 100, 100, 200..204)),
            Err(Error::first_chunk_out_of_range(2, 2)),
        ]
    );
}
