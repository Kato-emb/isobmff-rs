//! Throughput of the layers a fragmented file is laid down and read back through
//!
//! Three measurements stand side by side: what every composition of samples
//! costs through each layer, what the length of a fragment costs the writer, and
//! what the length of an arriving chunk costs the reader. Each of them reports
//! the bytes the samples carry, so the layers of one column are comparable, and
//! checks what it moved against the count its composition declares.

// Why not gathering the output: bytes drained into a growing buffer cost more to
// collect than the writer costs to produce them — a first attempt at this
// measurement spent 78% of its time there and reported the harness, not the
// library. Every byte written here goes through a buffer the caller reuses.

// Why not relaxing these in `clippy.toml`: `allow-unwrap-in-tests` reaches
// inside `#[cfg(test)]` alone, which a bench target is compiled without, so
// nothing short of an attribute here relaxes them.
#![allow(
    clippy::unwrap_used,
    clippy::arithmetic_side_effects,
    reason = "a bench that will not run is a bug in the bench, and its arithmetic is over lengths its own constants settle"
)]

use core::hint::black_box;
use criterion::{
    BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main,
    measurement::WallTime,
};

use isobmff::{
    BoxEvent, BoxReader, FileTypeBox, FragmentedReader, FragmentedWriter, MovieBox,
    MovieExtendsBox, MovieHeaderBox, Mp4EpochSeconds, Sample, SampleWriter, TrackExtendsBox,
};
use isobmff_test_support::{file_type, track};

/// Ticks a second the media of the benchmarked movies is timed in
const TIMESCALE: u32 = 90_000;

/// Ticks every sample of the benchmarked movies lasts
const SAMPLE_DURATION: u32 = 1_000;

/// Track the samples of the benchmarked movies belong to
const TRACK_ID: u32 = 1;

/// Buffer the written bytes are drained through, as a caller writing to a file would
const OUTPUT_BUFFER_LEN: usize = 64 * 1024;

/// Chunk the arriving bytes are handed over in, except where a benchmark varies it
const ARRIVING_CHUNK_LEN: usize = 64 * 1024;

/// A file to measure over: samples of one length, so many to a fragment, so many fragments
#[derive(Clone, Copy)]
struct Composition {
    /// How the composition is named in the report
    name: &'static str,
    /// Bytes every sample carries
    sample_len: usize,
    /// Samples one fragment holds
    samples_per_fragment: usize,
    /// Fragments the file holds
    fragment_count: usize,
}

impl Composition {
    /// Samples the whole file carries
    const fn sample_count(&self) -> usize {
        self.samples_per_fragment * self.fragment_count
    }

    /// Bytes the samples of the whole file carry, the header of no box counted
    const fn payload_len(&self) -> usize {
        self.sample_len * self.sample_count()
    }

    /// The samples of the file, fragment by fragment
    fn samples(&self) -> Vec<Vec<Sample>> {
        let mut decode_time = 0;

        (0..self.fragment_count)
            .map(|_| {
                (0..self.samples_per_fragment)
                    .map(|_| {
                        let sample = Sample::new(
                            TRACK_ID,
                            decode_time,
                            SAMPLE_DURATION,
                            None,
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
}

/// The compositions the first table reports, from long samples to short ones
const COMPOSITIONS: [Composition; 5] = [
    Composition {
        name: "video-64KiB-x30",
        sample_len: 64 * 1024,
        samples_per_fragment: 30,
        fragment_count: 32,
    },
    Composition {
        name: "video-64KiB-x300",
        sample_len: 64 * 1024,
        samples_per_fragment: 300,
        fragment_count: 3,
    },
    Composition {
        name: "audio-512B-x430",
        sample_len: 512,
        samples_per_fragment: 430,
        fragment_count: 276,
    },
    Composition {
        name: "audio-512B-x4300",
        sample_len: 512,
        samples_per_fragment: 4300,
        fragment_count: 27,
    },
    Composition {
        name: "tiny-64B-x1000",
        sample_len: 64,
        samples_per_fragment: 1000,
        fragment_count: 196,
    },
];

/// Samples one fragment holds, over the range the second table reports
const SAMPLES_PER_FRAGMENT: [usize; 7] = [1, 16, 30, 60, 120, 240, 960];

/// Samples the second table lays down, however they are split into fragments
const FRAGMENT_LENGTH_SAMPLE_COUNT: usize = 960;

/// Bytes every sample of the second table carries
const FRAGMENT_LENGTH_SAMPLE_LEN: usize = 64 * 1024;

/// Chunks the arriving bytes are handed over in, over the range the third table reports
const ARRIVING_CHUNK_LENS: [(&str, usize); 6] = [
    ("4MiB", 4 * 1024 * 1024),
    ("1MiB", 1024 * 1024),
    ("256KiB", 256 * 1024),
    ("64KiB", 64 * 1024),
    ("16KiB", 16 * 1024),
    ("4KiB", 4 * 1024),
];

/// Movie the benchmarked files continue in fragments
///
/// Every default the `trex` states is one no fragment falls back on, so what a
/// fragment writes is what the samples handed over stated.
fn movie() -> MovieBox {
    let epoch = Mp4EpochSeconds::from_seconds(0);

    MovieBox::new(
        MovieHeaderBox::new(epoch, epoch, TIMESCALE, 0, 2),
        vec![track(TRACK_ID)],
        MovieExtendsBox::new(vec![TrackExtendsBox::new(TRACK_ID, 9, 1, 1, u32::MAX)]),
    )
    .unwrap()
}

/// Drains what the writer has ready, and reports how many bytes that was
fn drained(writer: &mut FragmentedWriter, buffer: &mut [u8]) -> usize {
    let mut total = 0;

    loop {
        let written = writer.poll_output(buffer);

        if written == 0 {
            return total;
        }
        total += written;
        black_box(&buffer);
    }
}

/// Lays the fragments down as a whole file, and reports how many bytes it came to
fn fragmented_writer_file(
    file_type: FileTypeBox,
    movie: MovieBox,
    fragments: Vec<Vec<Sample>>,
) -> usize {
    let mut writer = FragmentedWriter::new();
    let mut buffer = [0; OUTPUT_BUFFER_LEN];
    let mut total = 0;

    writer.handle_file_type(file_type).unwrap();
    writer.handle_movie(movie).unwrap();

    for (position, samples) in fragments.into_iter().enumerate() {
        writer
            .begin_fragment(u32::try_from(position).unwrap() + 1)
            .unwrap();
        for sample in samples {
            writer.handle_sample(sample).unwrap();
        }
        writer.finish_fragment().unwrap();
        total += drained(&mut writer, &mut buffer);
    }
    writer.finish().unwrap();

    total + drained(&mut writer, &mut buffer)
}

/// Lays the fragments down as `moof` and `mdat` pairs, and reports the bytes they carry
///
/// The sample layer alone: the pairs are never framed as a file.
fn sample_writer_fragments(fragments: Vec<Vec<Sample>>) -> usize {
    let mut writer = SampleWriter::new();
    let mut total = 0;

    for (position, samples) in fragments.into_iter().enumerate() {
        writer
            .begin_fragment(u32::try_from(position).unwrap() + 1)
            .unwrap();
        for sample in samples {
            writer.handle_sample(sample).unwrap();
        }
        writer.finish_fragment().unwrap();

        while let Some((movie_fragment, media_data)) = writer.poll_fragment() {
            total += media_data.data().len();
            black_box(&movie_fragment);
        }
    }
    writer.finish().unwrap();

    total
}

/// Reads the samples off the file, and reports how many there were and what they carry
fn fragmented_reader_samples(file: &[u8], chunk_len: usize) -> (usize, usize) {
    let mut reader = FragmentedReader::new();
    let mut count = 0;
    let mut total = 0;
    let mut take = |reader: &mut FragmentedReader| {
        while let Some(sample) = reader.poll_sample() {
            count += 1;
            total += sample.data().len();
            black_box(&sample);
        }
    };

    for arriving in file.chunks(chunk_len) {
        reader.handle_input(arriving).unwrap();
        take(&mut reader);
    }
    reader.finish().unwrap();
    take(&mut reader);

    (count, total)
}

/// Frames the boxes of the file, and reports how many of them ended
///
/// The box layer alone: no payload is read into a value.
fn box_reader_boxes(file: &[u8], chunk_len: usize) -> usize {
    let mut reader = BoxReader::new();
    let mut count = 0;
    let mut take = |reader: &mut BoxReader| {
        while let Some(event) = reader.poll_event() {
            if matches!(event, BoxEvent::End) {
                count += 1;
            }
            black_box(&event);
        }
    };

    for arriving in file.chunks(chunk_len) {
        reader.handle_input(arriving).unwrap();
        take(&mut reader);
    }
    reader.finish().unwrap();
    take(&mut reader);

    count
}

/// The file the composition makes, laid down whole
fn written_file(composition: &Composition) -> Vec<u8> {
    let mut writer = FragmentedWriter::new();
    let mut file = Vec::new();
    let mut buffer = [0; OUTPUT_BUFFER_LEN];

    writer.handle_file_type(file_type()).unwrap();
    writer.handle_movie(movie()).unwrap();

    for (position, samples) in composition.samples().into_iter().enumerate() {
        writer
            .begin_fragment(u32::try_from(position).unwrap() + 1)
            .unwrap();
        for sample in samples {
            writer.handle_sample(sample).unwrap();
        }
        writer.finish_fragment().unwrap();
    }
    writer.finish().unwrap();

    loop {
        let written = writer.poll_output(&mut buffer);

        match buffer.get(..written) {
            Some([]) | None => return file,
            Some(bytes) => file.extend_from_slice(bytes),
        }
    }
}

/// Measures the two layers down and the two layers up, for every composition
fn composition(criterion: &mut Criterion<WallTime>) {
    let mut group = criterion.benchmark_group("composition");

    for composition in COMPOSITIONS {
        let file = written_file(&composition);
        let file_len = file.len();
        let payload_len = composition.payload_len();
        let sample_count = composition.sample_count();

        group.throughput(Throughput::Bytes(u64::try_from(payload_len).unwrap()));

        group.bench_function(
            BenchmarkId::new("fragmented_writer", composition.name),
            |bencher| {
                bencher.iter_batched(
                    || composition.samples(),
                    |fragments| {
                        assert_eq!(
                            fragmented_writer_file(file_type(), movie(), fragments),
                            file_len
                        )
                    },
                    BatchSize::PerIteration,
                );
            },
        );

        group.bench_function(
            BenchmarkId::new("sample_writer", composition.name),
            |bencher| {
                bencher.iter_batched(
                    || composition.samples(),
                    |fragments| assert_eq!(sample_writer_fragments(fragments), payload_len),
                    BatchSize::PerIteration,
                );
            },
        );

        group.bench_function(
            BenchmarkId::new("fragmented_reader", composition.name),
            |bencher| {
                bencher.iter(|| {
                    assert_eq!(
                        fragmented_reader_samples(&file, ARRIVING_CHUNK_LEN),
                        (sample_count, payload_len)
                    );
                });
            },
        );

        group.bench_function(
            BenchmarkId::new("box_reader", composition.name),
            |bencher| {
                bencher.iter(|| {
                    assert_eq!(
                        box_reader_boxes(&file, ARRIVING_CHUNK_LEN),
                        2 + 2 * composition.fragment_count
                    );
                });
            },
        );
    }

    group.finish();
}

/// Measures what splitting the same samples into longer or shorter fragments costs
fn fragment_length(criterion: &mut Criterion<WallTime>) {
    let mut group = criterion.benchmark_group("fragment_length");
    let payload_len = FRAGMENT_LENGTH_SAMPLE_COUNT * FRAGMENT_LENGTH_SAMPLE_LEN;

    group.throughput(Throughput::Bytes(u64::try_from(payload_len).unwrap()));

    for samples_per_fragment in SAMPLES_PER_FRAGMENT {
        let composition = Composition {
            name: "fragment-length",
            sample_len: FRAGMENT_LENGTH_SAMPLE_LEN,
            samples_per_fragment,
            fragment_count: FRAGMENT_LENGTH_SAMPLE_COUNT / samples_per_fragment,
        };
        let file_len = written_file(&composition).len();

        group.bench_function(
            BenchmarkId::new("sample_writer", samples_per_fragment),
            |bencher| {
                bencher.iter_batched(
                    || composition.samples(),
                    |fragments| assert_eq!(sample_writer_fragments(fragments), payload_len),
                    BatchSize::PerIteration,
                );
            },
        );

        group.bench_function(
            BenchmarkId::new("fragmented_writer", samples_per_fragment),
            |bencher| {
                bencher.iter_batched(
                    || composition.samples(),
                    |fragments| {
                        assert_eq!(
                            fragmented_writer_file(file_type(), movie(), fragments),
                            file_len
                        )
                    },
                    BatchSize::PerIteration,
                );
            },
        );
    }

    group.finish();
}

/// Measures what handing the same file over in longer or shorter chunks costs
fn chunk_length(criterion: &mut Criterion<WallTime>) {
    let mut group = criterion.benchmark_group("chunk_length");
    let composition = COMPOSITIONS[0];
    let file = written_file(&composition);
    let payload_len = composition.payload_len();
    let sample_count = composition.sample_count();
    let box_count = 2 + 2 * composition.fragment_count;
    let chunk_lens = [("whole-file", file.len())]
        .into_iter()
        .chain(ARRIVING_CHUNK_LENS);

    group.throughput(Throughput::Bytes(u64::try_from(payload_len).unwrap()));

    for (name, chunk_len) in chunk_lens {
        group.bench_function(BenchmarkId::new("fragmented_reader", name), |bencher| {
            bencher.iter(|| {
                assert_eq!(
                    fragmented_reader_samples(&file, chunk_len),
                    (sample_count, payload_len)
                );
            });
        });

        group.bench_function(BenchmarkId::new("box_reader", name), |bencher| {
            bencher.iter(|| assert_eq!(box_reader_boxes(&file, chunk_len), box_count));
        });
    }

    group.finish();
}

criterion_group!(benches, composition, fragment_length, chunk_length);
criterion_main!(benches);
