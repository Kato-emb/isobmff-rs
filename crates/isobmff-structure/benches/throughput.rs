//! Throughput of the layers a fragmented file is laid down and read back through
//!
//! Four measurements stand side by side: what every composition of samples costs
//! through each layer, what the length of a fragment costs the writer, what the
//! length of an arriving chunk costs the reader, and what the length of a box
//! costs the framing on its own. The first three report the bytes the samples
//! carry, so the layers of one column are comparable; the fourth reports boxes,
//! which is what its cost is paid by. Each of them checks what it moved against
//! what its input declares.

// Why not gathering the output: bytes drained into a growing buffer cost more to
// collect than the writer costs to produce them — a first attempt at this
// measurement spent 78% of its time there and reported the harness, not the
// library.

// Why not black_box the bytes a writer hands over: taking the address of an
// owned value forces it onto the stack and blocks its drop from being optimized,
// which the buffer this measurement used to drain through never paid — reading it
// against that buffer would charge the library for the harness.

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

use isobmff_boxes::{
    FileTypeBox, MovieBox, MovieExtendsBox, MovieHeaderBox, SampleFlags, TrackExtendsBox,
};
use isobmff_core::{BoxHeader, BoxType, Mp4EpochSeconds};
use isobmff_sample::{MovieFragmentWriter, Sample};
use isobmff_sequence::{BoxEvent, BoxReader, BoxWriter};
use isobmff_structure::{FragmentedReader, FragmentedWriter};
use isobmff_test_support::{EVERY_FIELD_AT_ITS_HIGHEST, file_type, track};

/// Ticks every sample of the benchmarked movies lasts
const SAMPLE_DURATION: u32 = 1_000;

/// Ticks a second the benchmarked movies are timed in
const TIMESCALE: u32 = 90_000;

/// Chunk the arriving bytes are handed over in, except where a benchmark varies it
const DEFAULT_ARRIVING_CHUNK_LEN: usize = 64 * 1024;

/// A file to measure over: samples of one length, so many to a fragment, so many fragments, over so many tracks
#[derive(Clone, Copy)]
struct Composition {
    /// Bytes every sample carries
    sample_len: usize,
    /// Samples one fragment holds
    samples_per_fragment: usize,
    /// Fragments the file holds
    fragment_count: usize,
    /// Tracks the samples take turns over, sample by sample
    track_count: usize,
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

    /// Boxes the file holds at its top level: the brands, the movie, then a pair per fragment
    const fn box_count(&self) -> usize {
        2 + 2 * self.fragment_count
    }

    /// The ids of the tracks, counting from one
    fn track_ids(&self) -> impl Iterator<Item = u32> {
        1..=u32::try_from(self.track_count).unwrap()
    }

    /// Movie the file continues in fragments
    ///
    /// Every default the `trex` states is one no fragment falls back on, so
    /// what a fragment writes is what the samples handed over stated.
    fn movie(&self) -> MovieBox {
        let epoch = Mp4EpochSeconds::from_seconds(0);

        MovieBox::new(
            MovieHeaderBox::new(
                epoch,
                epoch,
                TIMESCALE,
                0,
                u32::try_from(self.track_count).unwrap() + 1,
            ),
            self.track_ids().map(track).collect(),
            MovieExtendsBox::new(
                self.track_ids()
                    .map(|track_id| {
                        TrackExtendsBox::new(track_id, 9, 1, 1, EVERY_FIELD_AT_ITS_HIGHEST)
                    })
                    .collect(),
            ),
        )
        .unwrap()
    }

    /// The samples of the file, fragment by fragment, the tracks taking turns
    fn samples(&self) -> Vec<Vec<Sample>> {
        let mut decode_times = vec![0; self.track_count];

        (0..self.fragment_count)
            .map(|_| {
                (0..self.samples_per_fragment)
                    .map(|position| {
                        let track = position % self.track_count;
                        let decode_time = decode_times.get_mut(track).unwrap();
                        let sample = Sample::new(
                            u32::try_from(track).unwrap() + 1,
                            *decode_time,
                            SAMPLE_DURATION,
                            0,
                            SampleFlags::ZERO,
                            1,
                            vec![0xab; self.sample_len],
                        );
                        *decode_time += u64::from(SAMPLE_DURATION);

                        sample
                    })
                    .collect()
            })
            .collect()
    }
}

/// The compositions the first table reports, from long samples to short ones, then two tracks interleaved
const COMPOSITIONS: [(&str, Composition); 8] = [
    (
        "video-64KiB-x30",
        Composition {
            sample_len: 64 * 1024,
            samples_per_fragment: 30,
            fragment_count: 32,
            track_count: 1,
        },
    ),
    (
        "video-64KiB-x300",
        Composition {
            sample_len: 64 * 1024,
            samples_per_fragment: 300,
            fragment_count: 3,
            track_count: 1,
        },
    ),
    (
        "audio-512B-x430",
        Composition {
            sample_len: 512,
            samples_per_fragment: 430,
            fragment_count: 276,
            track_count: 1,
        },
    ),
    (
        "audio-512B-x4300",
        Composition {
            sample_len: 512,
            samples_per_fragment: 4300,
            fragment_count: 27,
            track_count: 1,
        },
    ),
    (
        "tiny-64B-x1000",
        Composition {
            sample_len: 64,
            samples_per_fragment: 1000,
            fragment_count: 196,
            track_count: 1,
        },
    ),
    (
        "audio-512B-x430-2-tracks",
        Composition {
            sample_len: 512,
            samples_per_fragment: 430,
            fragment_count: 276,
            track_count: 2,
        },
    ),
    (
        "audio-512B-x4300-2-tracks",
        Composition {
            sample_len: 512,
            samples_per_fragment: 4300,
            fragment_count: 27,
            track_count: 2,
        },
    ),
    (
        "tiny-64B-x1000-2-tracks",
        Composition {
            sample_len: 64,
            samples_per_fragment: 1000,
            fragment_count: 196,
            track_count: 2,
        },
    ),
];

/// The composition the second table splits into fragments seven ways
const FRAGMENT_LENGTH_BASE: Composition = Composition {
    sample_len: 64 * 1024,
    samples_per_fragment: 960,
    fragment_count: 1,
    track_count: 1,
};

/// The composition the third table hands over in seven chunk lengths
const CHUNK_LENGTH_BASE: Composition = Composition {
    sample_len: 64 * 1024,
    samples_per_fragment: 30,
    fragment_count: 32,
    track_count: 1,
};

/// Samples one fragment holds, over the range the second table reports
const SAMPLES_PER_FRAGMENT: [usize; 7] = [1, 16, 30, 60, 120, 240, 960];

/// Chunks the arriving bytes are handed over in, over the range the third table reports
const ARRIVING_CHUNK_LENS: [(&str, usize); 6] = [
    ("4MiB", 4 * 1024 * 1024),
    ("1MiB", 1024 * 1024),
    ("256KiB", 256 * 1024),
    ("64KiB", 64 * 1024),
    ("16KiB", 16 * 1024),
    ("4KiB", 4 * 1024),
];

/// Bytes the files of the fourth table come up to, whatever length the boxes they hold are
const BOX_LENGTH_FILE_LEN: usize = 12 * 1024 * 1024;

/// Payloads the boxes of the fourth table carry, over the range the fourth table reports
const BOX_PAYLOAD_LENS: [(&str, usize); 6] = [
    ("0B", 0),
    ("8B", 8),
    ("64B", 64),
    ("512B", 512),
    ("4KiB", 4 * 1024),
    ("64KiB", 64 * 1024),
];

/// Drains what the writer has ready, and reports how many bytes that was
fn drained(writer: &mut FragmentedWriter) -> usize {
    let mut total = 0;

    while let Some(written) = writer.poll_output() {
        total += written.len();
        black_box(written.len());
    }

    total
}

/// Lays the fragments down as a whole file, and reports how many bytes it came to
fn fragmented_writer_file(
    file_type: FileTypeBox,
    movie: MovieBox,
    fragments: Vec<Vec<Sample>>,
) -> usize {
    let mut writer = FragmentedWriter::new();
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
        total += drained(&mut writer);
    }
    writer.finish().unwrap();

    total + drained(&mut writer)
}

/// Lays the fragments down as `moof` and media data pairs, and reports the bytes they carry
///
/// The sample layer alone: the pairs are never framed as a file.
fn movie_fragment_writer_fragments(fragments: Vec<Vec<Sample>>) -> usize {
    let mut writer = MovieFragmentWriter::new();
    let mut total = 0;

    for (position, samples) in fragments.into_iter().enumerate() {
        writer
            .begin_fragment(u32::try_from(position).unwrap() + 1)
            .unwrap();
        for sample in samples {
            writer.handle_sample(sample).unwrap();
        }
        let (movie_fragment, media_data) = writer.finish_fragment().unwrap();

        total += media_data.len();
        black_box(&movie_fragment);
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

/// A file of `free` boxes of one payload length, and how many of them it holds
///
/// The file comes as near the length the table fixes as whole boxes reach.
fn free_boxes(payload_len: usize) -> (Vec<u8>, usize) {
    let header = BoxHeader::with_payload_len(
        BoxType::compact(*b"free"),
        u64::try_from(payload_len).unwrap(),
    )
    .unwrap();
    let mut scratch = [0; BoxHeader::MAX_ENCODED_LEN];
    let encoded = header.encode(&mut scratch).to_vec();
    let box_len = encoded.len() + payload_len;
    let box_count = BOX_LENGTH_FILE_LEN / box_len;
    let mut file = Vec::with_capacity(box_count * box_len);

    for _ in 0..box_count {
        file.extend_from_slice(&encoded);
        file.resize(file.len() + payload_len, 0xab);
    }

    (file, box_count)
}

/// The steps the boxes of the file are read back as
fn box_events(file: &[u8]) -> Vec<BoxEvent> {
    let mut reader = BoxReader::new();
    let mut events = Vec::new();
    let take = |reader: &mut BoxReader, events: &mut Vec<BoxEvent>| {
        while let Some(event) = reader.poll_event() {
            events.push(event);
        }
    };

    for arriving in file.chunks(DEFAULT_ARRIVING_CHUNK_LEN) {
        reader.handle_input(arriving).unwrap();
        take(&mut reader, &mut events);
    }
    reader.finish().unwrap();
    take(&mut reader, &mut events);

    events
}

/// Drains what the box writer has ready, and reports how many bytes that was
fn box_drained(writer: &mut BoxWriter) -> usize {
    let mut total = 0;

    while let Some(written) = writer.poll_output() {
        total += written.len();
        black_box(written.len());
    }

    total
}

/// Lays the events down as a file, and reports how many bytes it came to
///
/// The box layer alone: what an event carries is written as it stands.
fn box_writer_file(events: Vec<BoxEvent>) -> usize {
    let mut writer = BoxWriter::new();
    let mut total = 0;

    for event in events {
        writer.handle_event(event).unwrap();
        total += box_drained(&mut writer);
    }
    writer.finish().unwrap();

    total + box_drained(&mut writer)
}

/// The file the composition makes, laid down whole
fn written_file(composition: &Composition) -> Vec<u8> {
    let mut writer = FragmentedWriter::new();
    let mut file = Vec::with_capacity(composition.payload_len());
    let gather = |writer: &mut FragmentedWriter, file: &mut Vec<u8>| {
        while let Some(written) = writer.poll_output() {
            file.extend_from_slice(&written);
        }
    };

    writer.handle_file_type(file_type()).unwrap();
    writer.handle_movie(composition.movie()).unwrap();

    for (position, samples) in composition.samples().into_iter().enumerate() {
        writer
            .begin_fragment(u32::try_from(position).unwrap() + 1)
            .unwrap();
        for sample in samples {
            writer.handle_sample(sample).unwrap();
        }
        writer.finish_fragment().unwrap();
        gather(&mut writer, &mut file);
    }
    writer.finish().unwrap();
    gather(&mut writer, &mut file);

    file
}

/// Measures the two layers down and the two layers up, for every composition
fn composition(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("composition");

    for (name, composition) in COMPOSITIONS {
        let file = written_file(&composition);
        let file_len = file.len();
        let payload_len = composition.payload_len();
        let sample_count = composition.sample_count();
        let box_count = composition.box_count();

        group.throughput(Throughput::Bytes(u64::try_from(payload_len).unwrap()));

        group.bench_function(BenchmarkId::new("fragmented_writer", name), |bencher| {
            bencher.iter_batched(
                || (file_type(), composition.movie(), composition.samples()),
                |(file_type, movie, fragments)| {
                    assert_eq!(
                        fragmented_writer_file(file_type, movie, fragments),
                        file_len
                    )
                },
                BatchSize::PerIteration,
            );
        });

        group.bench_function(BenchmarkId::new("movie_fragment_writer", name), |bencher| {
            bencher.iter_batched(
                || composition.samples(),
                |fragments| assert_eq!(movie_fragment_writer_fragments(fragments), payload_len),
                BatchSize::PerIteration,
            );
        });

        group.bench_function(BenchmarkId::new("fragmented_reader", name), |bencher| {
            bencher.iter(|| {
                assert_eq!(
                    fragmented_reader_samples(&file, DEFAULT_ARRIVING_CHUNK_LEN),
                    (sample_count, payload_len)
                );
            });
        });

        group.bench_function(BenchmarkId::new("box_reader", name), |bencher| {
            bencher.iter(|| {
                assert_eq!(
                    box_reader_boxes(&file, DEFAULT_ARRIVING_CHUNK_LEN),
                    box_count
                );
            });
        });
    }

    group.finish();
}

/// Measures what splitting the same samples into longer or shorter fragments costs
fn fragment_length(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("fragment_length");

    group.throughput(Throughput::Bytes(
        u64::try_from(FRAGMENT_LENGTH_BASE.payload_len()).unwrap(),
    ));

    for samples_per_fragment in SAMPLES_PER_FRAGMENT {
        let composition = Composition {
            samples_per_fragment,
            fragment_count: FRAGMENT_LENGTH_BASE.sample_count() / samples_per_fragment,
            ..FRAGMENT_LENGTH_BASE
        };
        let payload_len = composition.payload_len();
        let file_len =
            fragmented_writer_file(file_type(), composition.movie(), composition.samples());

        group.bench_function(
            BenchmarkId::new("fragmented_writer", samples_per_fragment),
            |bencher| {
                bencher.iter_batched(
                    || (file_type(), composition.movie(), composition.samples()),
                    |(file_type, movie, fragments)| {
                        assert_eq!(
                            fragmented_writer_file(file_type, movie, fragments),
                            file_len
                        )
                    },
                    BatchSize::PerIteration,
                );
            },
        );

        group.bench_function(
            BenchmarkId::new("movie_fragment_writer", samples_per_fragment),
            |bencher| {
                bencher.iter_batched(
                    || composition.samples(),
                    |fragments| assert_eq!(movie_fragment_writer_fragments(fragments), payload_len),
                    BatchSize::PerIteration,
                );
            },
        );
    }

    group.finish();
}

/// Measures what handing the same file over in longer or shorter chunks costs
fn chunk_length(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("chunk_length");
    let file = written_file(&CHUNK_LENGTH_BASE);
    let payload_len = CHUNK_LENGTH_BASE.payload_len();
    let sample_count = CHUNK_LENGTH_BASE.sample_count();
    let box_count = CHUNK_LENGTH_BASE.box_count();
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

/// Measures what the length of a box costs the layer that frames it
///
/// The box layer alone, over a file of `free` boxes of one length: no samples are
/// laid down and none are read.
fn box_length(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("box_length");

    // Why not the default sample size: the shortest boxes come to a million and
    // a half of them to a file, which criterion would take a hundred samples of
    group.sample_size(10);

    for (name, payload_len) in BOX_PAYLOAD_LENS {
        let (file, box_count) = free_boxes(payload_len);
        let file_len = file.len();
        let events = box_events(&file);

        group.throughput(Throughput::Elements(u64::try_from(box_count).unwrap()));

        group.bench_function(BenchmarkId::new("box_reader", name), |bencher| {
            bencher.iter(|| {
                assert_eq!(
                    box_reader_boxes(&file, DEFAULT_ARRIVING_CHUNK_LEN),
                    box_count
                );
            });
        });

        group.bench_function(BenchmarkId::new("box_writer", name), |bencher| {
            bencher.iter_batched(
                || events.clone(),
                |events| assert_eq!(box_writer_file(events), file_len),
                BatchSize::PerIteration,
            );
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    composition,
    fragment_length,
    chunk_length,
    box_length
);
criterion_main!(benches);
