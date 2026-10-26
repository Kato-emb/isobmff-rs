//! Decoding of a `TrackRunBox` whose rows carry every field a sample can
//!
//! The benchmark sets `trun`, whose table is read once per sample, against a
//! loop that turns the same bytes into rows with no `Result` in the way, so
//! the cost of reading every row through a fallible field reader shows.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use isobmff_boxes::TrackRunBox;
use isobmff_core::BoxDecode;

/// Numbers of samples the table is measured at
const SAMPLE_COUNTS: [u32; 3] = [1_000, 100_000, 1_000_000];

/// Measures the decoding of `trun` against the loop with no `Result` in the way
fn track_run(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("trun");
    for sample_count in SAMPLE_COUNTS {
        let mut payload = vec![1, 0, 0x0f, 0];
        payload.extend_from_slice(&sample_count.to_be_bytes());
        payload.extend((0..sample_count).flat_map(|sample| {
            [1001, sample, 0x0101_0000, sample % 3]
                .into_iter()
                .flat_map(u32::to_be_bytes)
        }));
        group.throughput(Throughput::Elements(u64::from(sample_count)));
        group.bench_with_input(
            BenchmarkId::new("decode_payload", sample_count),
            &payload,
            |bencher, payload| {
                bencher.iter(|| TrackRunBox::decode_payload(black_box(payload)));
            },
        );
        group.bench_with_input(
            BenchmarkId::new("infallible_loop", sample_count),
            &payload,
            |bencher, payload| {
                bencher.iter(|| {
                    black_box(payload)
                        .get(8..)
                        .unwrap_or_default()
                        .as_chunks::<16>()
                        .0
                        .iter()
                        .map(|row| {
                            let (fields, _) = row.as_chunks::<4>();
                            let [duration, size, flags, offset] = [0, 1, 2, 3]
                                .map(|index| fields.get(index).copied().unwrap_or_default());
                            (
                                u32::from_be_bytes(duration),
                                u32::from_be_bytes(size),
                                u32::from_be_bytes(flags),
                                i32::from_be_bytes(offset),
                            )
                        })
                        .collect::<Vec<_>>()
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, track_run);
criterion_main!(benches);
