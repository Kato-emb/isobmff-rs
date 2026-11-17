//! Resolving a movie fragment while samples wait for many `imda`
//!
//! The structure-data reader checks the samples that wait for an `imda`,
//! with those of the box, against its limit before it resolves every box. It
//! keeps their count as it goes; adding up the samples that wait for each
//! identifier instead takes time that grows with the identifiers waited for,
//! on every box.

use std::hint::black_box;

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use isobmff::structure_data::StructureDataReader;
use isobmff_boxes::{
    DataEntry, DataEntrySeqNumImdaBox, MovieExtendsBox, MovieFragmentBox, TrackExtendsBox,
    TrackFragmentBox, TrackRunBox, TrackRunSamples,
};
use isobmff_test_support::fixtures::{movie, track};

/// A movie fragment of `sequence_number` of one sample of track 1, which counts from the payload of the `imda` of that identifier
fn moof(sequence_number: u32) -> MovieFragmentBox {
    let mut traf = TrackFragmentBox::default();
    traf.tfhd.track_id = 1;
    let mut trun = TrackRunBox::default();
    trun.data_offset = Some(4);
    trun.samples = TrackRunSamples::EmptyRows { sample_count: 1 };
    traf.trun = vec![trun];
    let mut moof = MovieFragmentBox::default();
    moof.mfhd.sequence_number = sequence_number;
    moof.traf = vec![traf];
    moof
}

/// A reader of a movie of track 1 in the `imda` of each sequence number, whose `waiting` fragments, of one sample each, wait for their `imda`
fn reader_waiting_for(waiting: u32) -> StructureDataReader {
    let mut trak = track(1, 20, &[], &[], &[]);
    trak.mdia.minf.dinf.dref.data_entries =
        vec![DataEntry::SeqNumImda(DataEntrySeqNumImdaBox::default())];
    let mut movie = movie([trak]);
    let mut trex = TrackExtendsBox::default();
    trex.track_id = 1;
    trex.default_sample_description_index = 1;
    trex.default_sample_duration = 20;
    trex.default_sample_size = 4;
    let mut mvex = MovieExtendsBox::default();
    mvex.trex = vec![trex];
    movie.mvex = Some(mvex);
    let mut reader = StructureDataReader::new(1 << 20, 1 << 16);
    let _ = reader.handle_movie(&movie).unwrap().count();
    for sequence_number in 1..=waiting {
        let _ = reader
            .handle_movie_fragment(&moof(sequence_number), 0)
            .unwrap()
            .count();
    }
    reader
}

fn handle_movie_fragment(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("handle_movie_fragment_waiting_for_imda");
    for waiting in [1 << 10, 1 << 13, 1 << 16] {
        let reader = reader_waiting_for(waiting);
        let next = moof(waiting + 1);
        group.bench_with_input(
            BenchmarkId::new("waiting count kept", waiting),
            &next,
            |bencher, next| {
                bencher.iter_batched(
                    || reader.clone(),
                    |mut reader| {
                        let resolved = reader
                            .handle_movie_fragment(black_box(next), 0)
                            .unwrap()
                            .count();
                        (reader, resolved)
                    },
                    BatchSize::LargeInput,
                );
            },
        );
    }
    group.finish();
}

criterion_group!(benches, handle_movie_fragment);
criterion_main!(benches);
