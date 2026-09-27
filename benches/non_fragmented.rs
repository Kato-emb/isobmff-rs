//! Throughput of the layers a non-fragmented file is laid down and read back through
//!
//! Two measurements stand side by side with the fragmented ones of
//! `fragmented.rs`: what every composition of samples costs the writer, and the
//! reader with the movie lying before its media data or after it, and what the
//! number of samples one movie declares costs the reader, which holds the
//! extents of them all at once, and the resolver on its own. The first reports
//! the bytes the samples carry, so its columns read against the fragmented
//! ones; the second reports samples, which is what its cost is paid by. Each
//! of them checks what it moved against what its input declares.
//!
//! Every group also carries harness rows, which do what a row does short of
//! calling the library: the writer's harness sets up the input the writer row
//! is handed and hands it back, the reader's of each layout hands the file over
//! chunk by chunk and then fetched want by want, the wants recorded from the
//! reader beforehand, to nothing, and the resolver's is handed the movie. What
//! a row costs the library is its value less the harness row of its side. The
//! writer row hands back what the writer handed over, so its disposal is left
//! out of the row as the harness leaves out that of the input.

// Why not gathering the output into one buffer, why not black_box the bytes a
// writer hands over, and why not dropping the input or the output in the
// routine: the notes at the head of `fragmented.rs` hold for this file too.

use core::hint::black_box;
use core::ops::Range;
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

use isobmff::boxes::{FileTypeBox, MovieBox, SampleFlags};
use isobmff::sample::Sample;
use isobmff::sample::sample_table::sample_extents;
use isobmff::sequence::EventBytes;
use isobmff::structure::{NonFragmentedReader, NonFragmentedWriter};
use isobmff_test_support::{SAMPLE_DURATION, file_type, non_fragmented_file, unfragmented_movie};

/// Track the samples of the benchmarked movies belong to
const TRACK_ID: u32 = 1;

/// Chunk the arriving bytes are handed over in
const ARRIVING_CHUNK_LEN: usize = 64 * 1024;

/// Bytes of input from which criterion sets up a row's inputs in the smaller batches of `BatchSize::LargeInput`
const LARGE_INPUT_LEN: usize = 32 * 1024 * 1024;

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
                            SampleFlags::ZERO,
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

/// How many inputs criterion sets up ahead of a routine that is handed `input_len` bytes
const fn batch_size(input_len: usize) -> BatchSize {
    if input_len < LARGE_INPUT_LEN {
        BatchSize::SmallInput
    } else {
        BatchSize::LargeInput
    }
}

/// Hands the file over chunk by chunk and then fetched want by want, to nothing, as a reader row would to its reader
fn handed_over(file: &[u8], wants: &[Range<u64>], fetch_len: usize) {
    for arriving in file.chunks(ARRIVING_CHUNK_LEN) {
        black_box(arriving);
    }
    for wanted in wants {
        black_box(fetched(file, wanted, fetch_len));
    }
}

/// The bytes fetched for `wanted`: from its start to its end or to `fetch_len` bytes on, whichever is further, as far as the file goes
fn fetched<'file>(file: &'file [u8], wanted: &Range<u64>, fetch_len: usize) -> &'file [u8] {
    let start = usize::try_from(wanted.start).unwrap();
    let end = usize::try_from(wanted.end).unwrap();

    file.get(start..end.max(start + fetch_len).min(file.len()))
        .unwrap()
}

/// Drains what the writer has ready into `outputs`, and reports how many bytes that was
fn drained(writer: &mut NonFragmentedWriter, outputs: &mut Vec<EventBytes>) -> usize {
    let mut total = 0;

    while let Some(written) = writer.poll_output() {
        total += written.len();
        outputs.push(written);
    }

    total
}

/// Lays the chunks down as a whole file, and hands back how many bytes it came to and the outputs that carry them
fn non_fragmented_writer_file(
    file_type: FileTypeBox,
    movie: MovieBox,
    chunks: Vec<Vec<Sample>>,
) -> (usize, Vec<EventBytes>) {
    let mut writer = NonFragmentedWriter::new();
    let mut outputs = Vec::new();
    let mut total = 0;

    writer.handle_file_type(file_type).unwrap();
    writer.handle_movie(movie).unwrap();

    for samples in chunks {
        writer.begin_chunk().unwrap();
        for sample in samples {
            writer.handle_sample(sample).unwrap();
        }
        total += drained(&mut writer, &mut outputs);
    }
    writer.finish().unwrap();
    total += drained(&mut writer, &mut outputs);

    (total, outputs)
}

/// Reads the samples off the file, and reports how many there were and what they carry
///
/// The file is handed over in order, a chunk at a time, and then whatever the
/// reader still wants is fetched: nothing where the movie lay first, and every
/// sample where it lay last.
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
        reader
            .handle_data(wanted.start, fetched(file, &wanted, fetch_len))
            .unwrap();
        take(&mut reader);
    }
    reader.finish().unwrap();
    take(&mut reader);

    (count, total)
}

/// The extents the reader wants once the file has been handed over, in the order it wants them
fn wants_of(file: &[u8], fetch_len: usize) -> Vec<Range<u64>> {
    let mut reader = NonFragmentedReader::new();
    let mut wants = Vec::new();
    let take = |reader: &mut NonFragmentedReader| while reader.poll_sample().is_some() {};

    for arriving in file.chunks(ARRIVING_CHUNK_LEN) {
        reader.handle_input(arriving).unwrap();
        take(&mut reader);
    }
    while let Some(wanted) = reader.wanted_extent() {
        reader
            .handle_data(wanted.start, fetched(file, &wanted, fetch_len))
            .unwrap();
        take(&mut reader);
        wants.push(wanted);
    }

    wants
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
        let movie_first = composition.file(true);
        let movie_last = composition.file(false);
        let file_len = movie_last.len();
        let payload_len = composition.payload_len();
        let sample_count = composition.sample_count();

        group.throughput(Throughput::BytesDecimal(
            u64::try_from(payload_len).unwrap(),
        ));

        group.bench_function(BenchmarkId::new("harness/writer", name), |bencher| {
            bencher.iter_batched(
                || (file_type(), unfragmented_movie(), composition.samples()),
                black_box,
                batch_size(payload_len),
            );
        });

        group.bench_function(BenchmarkId::new("non_fragmented_writer", name), |bencher| {
            bencher.iter_batched(
                || (file_type(), unfragmented_movie(), composition.samples()),
                |(file_type, movie, chunks)| {
                    let (written_len, outputs) =
                        non_fragmented_writer_file(file_type, movie, chunks);
                    assert_eq!(written_len, file_len);
                    outputs
                },
                batch_size(payload_len),
            );
        });

        for (layout, file) in [("movie-first", &movie_first), ("movie-last", &movie_last)] {
            let wants = wants_of(file, ARRIVING_CHUNK_LEN);

            group.bench_function(
                BenchmarkId::new(format!("harness/reader/{layout}"), name),
                |bencher| {
                    bencher.iter(|| handed_over(file, &wants, ARRIVING_CHUNK_LEN));
                },
            );

            group.bench_function(
                BenchmarkId::new(format!("non_fragmented_reader/{layout}"), name),
                |bencher| {
                    bencher.iter(|| {
                        assert_eq!(
                            non_fragmented_reader_samples(file, ARRIVING_CHUNK_LEN),
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
        let movie_first = composition.file(true);
        let movie_last = composition.file(false);

        group.throughput(Throughput::Elements(u64::try_from(sample_count).unwrap()));

        for (layout, file) in [("movie-first", &movie_first), ("movie-last", &movie_last)] {
            let wants = wants_of(file, 0);

            group.bench_function(
                BenchmarkId::new(format!("harness/reader/{layout}"), sample_count),
                |bencher| {
                    bencher.iter(|| handed_over(file, &wants, 0));
                },
            );

            group.bench_function(
                BenchmarkId::new(format!("non_fragmented_reader/{layout}"), sample_count),
                |bencher| {
                    bencher.iter(|| {
                        assert_eq!(
                            non_fragmented_reader_samples(file, 0),
                            (sample_count, payload_len)
                        );
                    });
                },
            );
        }

        let movie = movie_of(&movie_first);

        group.bench_function(
            BenchmarkId::new("harness/sample_table_extents", sample_count),
            |bencher| {
                bencher.iter(|| black_box(&movie));
            },
        );

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
