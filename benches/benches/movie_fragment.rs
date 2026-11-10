//! Taking the samples of one movie fragment into the movie fragment writer
//!
//! The writer bounds each fragment by where its media data ends, the end of
//! the sample that ends last, on every sample. It keeps that end as it goes;
//! finding it among the samples it holds on every sample instead takes time
//! quadratic in the number of samples.

use std::hint::black_box;

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use isobmff::structure_data::{MovieFragmentWriter, TrackSampleDescription};
use isobmff::{SampleLocation, SampleProperties};
use isobmff_boxes::{
    DataEntry, DataEntryUrlBox, DataReferenceBox, SampleDescription, SampleDescriptionBox,
    SampleEntry, SampleFlags,
};

const SAMPLE_DURATION: u32 = 512;
const SAMPLE_SIZE: u32 = 100;

/// The samples of one fragment of one track, `count` of them laid one after another in its media data
fn fragment(count: u64) -> Vec<SampleLocation> {
    (0..count)
        .map(|index| SampleLocation {
            properties: SampleProperties {
                track_id: 1,
                decoding_time: index * u64::from(SAMPLE_DURATION),
                sample_duration: SAMPLE_DURATION,
                sample_composition_time_offset: 0,
                sample_flags: SampleFlags::default(),
                sample_description_index: 1,
            },
            offset: index * u64::from(SAMPLE_SIZE),
            size: SAMPLE_SIZE,
        })
        .collect()
}

fn handle_location(criterion: &mut Criterion) {
    let mut entry = SampleEntry::default();
    entry.data_reference_index = 1;
    let mut stsd = SampleDescriptionBox::default();
    stsd.sample_descriptions = vec![SampleDescription::Typed(entry)];
    let mut dref = DataReferenceBox::default();
    dref.data_entries = vec![DataEntry::Url(DataEntryUrlBox::default())];
    let track = TrackSampleDescription {
        track_id: 1,
        stsd: &stsd,
        dref: &dref,
    };
    let mut group = criterion.benchmark_group("movie_fragment_handle_location");
    group.sample_size(10);
    for count in [1 << 10, 1 << 14, 1 << 17] {
        let samples = fragment(count);
        group.bench_with_input(
            BenchmarkId::new("media data end kept", count),
            &samples,
            |bencher, samples| {
                bencher.iter_batched(
                    || MovieFragmentWriter::new(&[track], count),
                    |mut writer| {
                        for &sample in samples {
                            writer.handle_location(black_box(sample)).unwrap();
                        }
                        writer
                    },
                    BatchSize::LargeInput,
                );
            },
        );
    }
    group.finish();
}

criterion_group!(benches, handle_location);
criterion_main!(benches);
