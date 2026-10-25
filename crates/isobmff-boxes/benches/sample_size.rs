//! Decoding of a `SampleSizeBox` that holds a table of a size per sample
//!
//! The benchmark sets `stsz`, whose table is read once per sample, against a
//! loop that turns the same bytes into sizes with no `Result` in the way, so
//! the cost of reading every entry through a fallible field reader shows.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use isobmff_boxes::SampleSizeBox;
use isobmff_core::BoxDecode;

/// Numbers of samples the table is measured at
const SAMPLE_COUNTS: [u32; 3] = [1_000, 100_000, 1_000_000];

/// Measures the decoding of `stsz` against the loop with no `Result` in the way
fn sample_size(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("stsz");
    for sample_count in SAMPLE_COUNTS {
        let mut payload = vec![0; 8];
        payload.extend_from_slice(&sample_count.to_be_bytes());
        payload.extend((0..sample_count).flat_map(|size| size.to_be_bytes()));
        group.throughput(Throughput::Elements(u64::from(sample_count)));
        group.bench_with_input(
            BenchmarkId::new("decode_payload", sample_count),
            &payload,
            |bencher, payload| {
                bencher.iter(|| SampleSizeBox::decode_payload(black_box(payload)));
            },
        );
        group.bench_with_input(
            BenchmarkId::new("infallible_loop", sample_count),
            &payload,
            |bencher, payload| {
                bencher.iter(|| {
                    black_box(payload)
                        .get(12..)
                        .unwrap_or_default()
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .copied()
                        .map(u32::from_be_bytes)
                        .collect::<Vec<_>>()
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, sample_size);
criterion_main!(benches);
