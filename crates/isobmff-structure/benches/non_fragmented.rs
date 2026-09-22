//! Throughput of the layers a non-fragmented file is laid down and read back through
//!
//! Two measurements stand side by side with the fragmented ones of
//! `throughput.rs`: what every composition of samples costs the writer, and the
//! reader with the movie lying before its media data or after it, and what the
//! number of samples one movie declares costs the reader, which holds the
//! extents of them all at once, and the resolver on its own. The first reports
//! the bytes the samples carry, so its columns read against the fragmented
//! ones; the second reports samples, which is what its cost is paid by. Each
//! of them checks what it moved against what its input declares.

// Why not gathering the output, and why not black_box the bytes a writer hands
// over: the notes at the head of `throughput.rs` hold for this file too.

// Why not relaxing these in `clippy.toml`: `allow-unwrap-in-tests` reaches
// inside `#[cfg(test)]` alone, which a bench target is compiled without, so
// nothing short of an attribute here relaxes them.
#![allow(
    clippy::unwrap_used,
    clippy::arithmetic_side_effects,
    reason = "a bench that will not run is a bug in the bench, and its arithmetic is over lengths its own constants settle"
)]

use core::hint::black_box;
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

use isobmff_boxes::{FileTypeBox, MovieBox};
use isobmff_sample::Sample;
use isobmff_sample::sample_table::sample_extents;
use isobmff_structure::{NonFragmentedReader, NonFragmentedWriter};
use isobmff_test_support::{SAMPLE_DURATION, file_type, non_fragmented_file, unfragmented_movie};

/// Track the samples of the benchmarked movies belong to
const TRACK_ID: u32 = 1;

/// Chunk the arriving bytes are handed over in
const ARRIVING_CHUNK_LEN: usize = 64 * 1024;

/// A file to measure over: samples of one length, so many to a chunk, so many chunks
#[derive(Clone, Copy)]
struct Composition {
    /// Bytes every sample carries
    sample_len: usize,
    /// Samples one chunk holds
    samples_per_chunk: usize,
    /// Chunks the file holds
    chunk_count: usize,
}

impl Composition {
    /// Samples the whole file carries
    const fn sample_count(&self) -> usize {
        self.samples_per_chunk * self.chunk_count
    }

    /// Bytes the samples of the whole file carry, the header of no box counted
    const fn payload_len(&self) -> usize {
        self.sample_len * self.sample_count()
    }

    /// The samples of the file, chunk by chunk
    fn samples(&self) -> Vec<Vec<Sample>> {
        let mut decode_time = 0;

        (0..self.chunk_count)
            .map(|_| {
                (0..self.samples_per_chunk)
                    .map(|_| {
                        let sample = Sample::new(
                            TRACK_ID,
                            decode_time,
                            SAMPLE_DURATION,
                            0,
                            0,
                            1,
                            vec![0xab; self.sample_len],
                        );
                        decode_time += u64::from(SAMPLE_DURATION);

                        sample
                    })
                    .collect()
            })
            .collect()
    }

    /// The file the composition makes, its movie lying first or last
    fn file(&self, movie_first: bool) -> Vec<u8> {
        let sample = vec![0xab; self.sample_len];
        let chunk = vec![sample.as_slice(); self.samples_per_chunk];

        non_fragmented_file(&vec![chunk.as_slice(); self.chunk_count], movie_first)
    }
}

/// The compositions the first table reports: the rows of the fragmented one, a chunk standing where a fragment stood
const COMPOSITIONS: [(&str, Composition); 5] = [
    (
        "video-64KiB-x30",
        Composition {
            sample_len: 64 * 1024,
            samples_per_chunk: 30,
            chunk_count: 32,
        },
    ),
    (
        "video-64KiB-x300",
        Composition {
            sample_len: 64 * 1024,
            samples_per_chunk: 300,
            chunk_count: 3,
        },
    ),
    (
        "audio-512B-x430",
        Composition {
            sample_len: 512,
            samples_per_chunk: 430,
            chunk_count: 276,
        },
    ),
    (
        "audio-512B-x4300",
        Composition {
            sample_len: 512,
            samples_per_chunk: 4300,
            chunk_count: 27,
        },
    ),
    (
        "tiny-64B-x1000",
        Composition {
            sample_len: 64,
            samples_per_chunk: 1000,
            chunk_count: 196,
        },
    ),
];

/// Bytes every sample of the second table carries
const SAMPLE_COUNT_SAMPLE_LEN: usize = 64;

/// Samples one chunk of the second table holds
const SAMPLE_COUNT_SAMPLES_PER_CHUNK: usize = 100;

/// Samples one movie declares, over the range the second table reports
const SAMPLE_COUNTS: [usize; 3] = [1_000, 10_000, 100_000];

/// The layouts of the movie the reader is measured over, by name and by whether the movie lies first
const LAYOUTS: [(&str, bool); 2] = [("movie-first", true), ("movie-last", false)];

/// Drains what the writer has ready, and reports how many bytes that was
fn drained(writer: &mut NonFragmentedWriter) -> usize {
    let mut total = 0;

    while let Some(written) = writer.poll_output() {
        total += written.len();
        black_box(written.len());
    }

    total
}

/// Lays the chunks down as a whole file, and reports how many bytes it came to
fn non_fragmented_writer_file(
    file_type: FileTypeBox,
    movie: MovieBox,
    chunks: Vec<Vec<Sample>>,
) -> usize {
    let mut writer = NonFragmentedWriter::new();
    let mut total = 0;

    writer.handle_file_type(file_type).unwrap();
    writer.handle_movie(movie).unwrap();

    for samples in chunks {
        writer.begin_chunk().unwrap();
        for sample in samples {
            writer.handle_sample(sample).unwrap();
        }
        total += drained(&mut writer);
    }
    writer.finish().unwrap();

    total + drained(&mut writer)
}

/// Reads the samples off the file, and reports how many there were and what they carry
///
/// The file is handed over in order, a chunk at a time, and then whatever the
/// reader still wants is fetched: nothing where the movie lay first, and every
/// sample where it lay last, each fetch reaching from the want to its end or to
/// `fetch_len` bytes on, whichever is further, as far as the file goes.
fn non_fragmented_reader_samples(file: &[u8], fetch_len: usize) -> (usize, usize) {
    let mut reader = NonFragmentedReader::new();
    let mut count = 0;
    let mut total = 0;
    let mut take = |reader: &mut NonFragmentedReader| {
        while let Some(sample) = reader.poll_sample() {
            count += 1;
            total += sample.data().len();
            black_box(&sample);
        }
    };

    for arriving in file.chunks(ARRIVING_CHUNK_LEN) {
        reader.handle_input(arriving).unwrap();
        take(&mut reader);
    }
    while let Some(wanted) = reader.wanted_extent() {
        let start = usize::try_from(wanted.start).unwrap();
        let end = usize::try_from(wanted.end).unwrap();
        let fetched = file
            .get(start..end.max(start + fetch_len).min(file.len()))
            .unwrap();

        reader.handle_data(wanted.start, fetched).unwrap();
        take(&mut reader);
    }
    reader.finish().unwrap();
    take(&mut reader);

    (count, total)
}

/// The movie `file` declares, read off it
fn movie_of(file: &[u8]) -> MovieBox {
    let mut reader = NonFragmentedReader::new();

    reader.handle_input(file).unwrap();

    reader.movie().cloned().unwrap()
}

/// Resolves the samples the movie declares, and reports how many there were
///
/// The resolution layer alone: no sample is gathered.
fn sample_table_extents(movie: &MovieBox) -> usize {
    sample_extents(movie)
        .inspect(|extent| {
            black_box(extent.as_ref().unwrap());
        })
        .count()
}

/// Measures the writer and the reader, the movie first and last, for every composition
fn composition(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("non_fragmented_composition");

    for (name, composition) in COMPOSITIONS {
        let file_len = composition.file(false).len();
        let payload_len = composition.payload_len();
        let sample_count = composition.sample_count();

        group.throughput(Throughput::Bytes(u64::try_from(payload_len).unwrap()));

        group.bench_function(BenchmarkId::new("non_fragmented_writer", name), |bencher| {
            bencher.iter_batched(
                || (file_type(), unfragmented_movie(), composition.samples()),
                |(file_type, movie, chunks)| {
                    assert_eq!(
                        non_fragmented_writer_file(file_type, movie, chunks),
                        file_len
                    )
                },
                BatchSize::PerIteration,
            );
        });

        for (layout, movie_first) in LAYOUTS {
            let file = composition.file(movie_first);

            group.bench_function(
                BenchmarkId::new(format!("non_fragmented_reader/{layout}"), name),
                |bencher| {
                    bencher.iter(|| {
                        assert_eq!(
                            non_fragmented_reader_samples(&file, ARRIVING_CHUNK_LEN),
                            (sample_count, payload_len)
                        );
                    });
                },
            );
        }
    }

    group.finish();
}

/// Measures what the number of samples a movie declares costs the reader and the resolver
///
/// The movie is read in both layouts; lying last it is read want by want, each
/// fetch the one sample the reader asks for, which is the most calls a file
/// makes on the extents held.
fn sample_count(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("non_fragmented_sample_count");

    // Why not the default sample size: the largest movie read want by want
    // takes seconds a pass, which criterion would take a hundred passes of
    group.sample_size(10);

    for sample_count in SAMPLE_COUNTS {
        let composition = Composition {
            sample_len: SAMPLE_COUNT_SAMPLE_LEN,
            samples_per_chunk: SAMPLE_COUNT_SAMPLES_PER_CHUNK,
            chunk_count: sample_count / SAMPLE_COUNT_SAMPLES_PER_CHUNK,
        };
        let payload_len = composition.payload_len();

        group.throughput(Throughput::Elements(u64::try_from(sample_count).unwrap()));

        for (layout, movie_first) in LAYOUTS {
            let file = composition.file(movie_first);

            group.bench_function(
                BenchmarkId::new(format!("non_fragmented_reader/{layout}"), sample_count),
                |bencher| {
                    bencher.iter(|| {
                        assert_eq!(
                            non_fragmented_reader_samples(&file, 0),
                            (sample_count, payload_len)
                        );
                    });
                },
            );
        }

        let movie = movie_of(&composition.file(true));

        group.bench_function(
            BenchmarkId::new("sample_table_extents", sample_count),
            |bencher| {
                bencher.iter(|| assert_eq!(sample_table_extents(&movie), sample_count));
            },
        );
    }

    group.finish();
}

criterion_group!(benches, composition, sample_count);
criterion_main!(benches);
