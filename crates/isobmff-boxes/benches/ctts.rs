//! Decoding of a `CompositionOffsetBox` that holds a run of samples per entry
//!
//! The benchmark sets `ctts`, whose table is read once per sample, against a
//! loop that turns the same bytes into entries with no `Result` in the way, so
//! the cost of reading every entry through a fallible field reader shows.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use isobmff_boxes::CompositionOffsetBox;
use isobmff_core::BoxDecode;

/// Numbers of entries the table is measured at
const ENTRY_COUNTS: [u32; 3] = [1_000, 100_000, 1_000_000];

/// Measures the decoding of `ctts` against the loop with no `Result` in the way
fn composition_offset(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("ctts");
    for entry_count in ENTRY_COUNTS {
        let mut payload = vec![1, 0, 0, 0];
        payload.extend_from_slice(&entry_count.to_be_bytes());
        payload.extend(
            (0..entry_count)
                .flat_map(|entry| [1, entry % 3].into_iter().flat_map(u32::to_be_bytes)),
        );
        group.throughput(Throughput::Elements(u64::from(entry_count)));
        group.bench_with_input(
            BenchmarkId::new("decode_payload", entry_count),
            &payload,
            |bencher, payload| {
                bencher.iter(|| CompositionOffsetBox::decode_payload(black_box(payload)));
            },
        );
        group.bench_with_input(
            BenchmarkId::new("infallible_loop", entry_count),
            &payload,
            |bencher, payload| {
                bencher.iter(|| {
                    black_box(payload)
                        .get(8..)
                        .unwrap_or_default()
                        .as_chunks::<8>()
                        .0
                        .iter()
                        .map(|entry| {
                            let (fields, _) = entry.as_chunks::<4>();
                            let [count, offset] =
                                [0, 1].map(|index| fields.get(index).copied().unwrap_or_default());
                            (
                                u32::from_be_bytes(count),
                                i64::from(i32::from_be_bytes(offset)),
                            )
                        })
                        .collect::<Vec<_>>()
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, composition_offset);
criterion_main!(benches);
