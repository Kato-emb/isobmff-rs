//! Taking the samples of a track into the sample table writer
//!
//! The writer checks each sample against the decoding time the durations
//! before it add up to. It keeps that time as it goes; summing the durations
//! it holds on every sample instead, measured here beside it, takes time
//! quadratic in the number of samples.

use std::hint::black_box;

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use isobmff::structure_data::SampleTableWriter;
use isobmff::{SampleLocation, SampleProperties};
use isobmff_boxes::{
    DataEntry, DataEntryUrlBox, DataReferenceBox, SampleDescription, SampleDescriptionBox,
    SampleEntry, SampleFlags,
};

const SAMPLE_DURATION: u32 = 512;
const SAMPLE_SIZE: u32 = 100;

/// The samples of one track, `count` of them laid one after another
fn track(count: u64) -> Vec<SampleLocation> {
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

/// Takes `samples` checking each against the sum of the durations taken before it
fn take_summing(samples: &[SampleLocation]) -> Vec<u32> {
    let mut durations = Vec::new();
    for sample in samples {
        let expected: u64 = durations.iter().copied().map(u64::from).sum();
        assert_eq!(sample.properties.decoding_time, expected);
        durations.push(sample.properties.sample_duration);
    }
    durations
}

fn handle_location(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("sample_table_handle_location");
    group.sample_size(10);
    for count in [1 << 14, 1 << 16, 1 << 18] {
        let samples = track(count);
        group.bench_with_input(
            BenchmarkId::new("decoding time kept", count),
            &samples,
            |bencher, samples| {
                let mut entry = SampleEntry::default();
                entry.data_reference_index = 1;
                let mut stsd = SampleDescriptionBox::default();
                stsd.sample_descriptions = vec![SampleDescription::Typed(entry)];
                let mut dref = DataReferenceBox::default();
                dref.data_entries = vec![DataEntry::Url(DataEntryUrlBox::default())];
                bencher.iter_batched(
                    || SampleTableWriter::new(&stsd, &dref, count),
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
        group.bench_with_input(
            BenchmarkId::new("durations summed", count),
            &samples,
            |bencher, samples| bencher.iter(|| take_summing(black_box(samples))),
        );
    }
    group.finish();
}

criterion_group!(benches, handle_location);
criterion_main!(benches);
