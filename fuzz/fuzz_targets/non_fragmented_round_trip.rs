//! Round-trip properties of [`NonFragmentedWriter`] against [`NonFragmentedReader`]
//!
//! One run lays samples down as a non-fragmented file, chunk by chunk, reads
//! that file back, and checks three properties of the same input:
//!
//! 1. no call panics: a writer that refuses a sample reports that same failure
//!    for every call after it, still hands over the bytes it had laid down, and
//!    those bytes carry no movie, which the reader says of them
//! 2. the samples are read back as they were handed over, the file over in
//!    order and cut anywhere: a chunk lies where it was opened, so the movie
//!    declares the samples in the order they were handed over and the reader
//!    meets them in it, whether the media data arrives before the movie has
//!    claimed it and is fetched again, or a movie moved before the media data
//!    claims it as it comes — the samples of the moved file compared by their
//!    bytes alone, the fixture laying one track out
//! 3. the reader does not reject the file the writer laid down
//!
//! A sample carries a byte at least: an extent naming no bytes is whole where it
//! is held, and comes out where the extents held before it clear, which the
//! order of the file does not settle.

#![no_main]

use isobmff::boxes::{HeaderDuration, MovieBox, MovieHeaderBox, SampleFlags};
use isobmff::core::Mp4EpochSeconds;
use isobmff::sample::Sample;
use isobmff::structure::{Error, ErrorKind, NonFragmentedReader, NonFragmentedWriter};
use isobmff_test_support::{file_type, non_fragmented_file, track};
use libfuzzer_sys::arbitrary::{self, Arbitrary};
use libfuzzer_sys::fuzz_target;

/// The tracks the samples of a run are laid out over, by the id the movie declares each under
const TRACK_IDS: [u32; 2] = [1, 2];

/// Chunks one run lays out at most
const MAX_CHUNKS: usize = 8;

/// Samples one chunk carries at most
const MAX_SAMPLES: usize = 8;

/// The `stsd` entry every sample is described by, the one entry each track carries
const SAMPLE_DESCRIPTION_INDEX: u32 = 1;

/// Ticks a second the movie is timed in
const TIMESCALE: u32 = 1_000;

/// The id the movie hands out to the track declared next
const NEXT_TRACK_ID: u32 = 3;

/// Input of one run: the samples of each chunk, and the bytes they are carried as
#[derive(Arbitrary, Debug)]
struct Input<'bytes> {
    /// Chunks laid out, in the order they are opened
    chunks: Vec<Chunk>,
    /// Which sample, counted over the whole run, states a decode time of its own
    decode_time_broken_at: Option<u8>,
    /// Bytes the file is cut into as it is read back, one more than this
    cut_length: u8,
    // Why not put `sample_data` first, and why a slice: only the last field is
    // handed what is left, and only `&[u8]` takes it verbatim — a `Vec<u8>` reads
    // a byte of its own before each element, so a seed stops at its first even
    // byte.
    sample_data: &'bytes [u8],
}

/// One chunk: which track it holds, and the samples handed over for it
#[derive(Arbitrary, Debug)]
struct Chunk {
    /// Whether the chunk belongs to the second of the tracks the movie declares
    second_track: bool,
    samples: Vec<Stated>,
}

/// One sample as the caller states it, over the bytes it is carried as
#[derive(Arbitrary, Debug)]
struct Stated {
    duration: u16,
    /// Bytes of the sample data this sample takes, one more than this
    length: u8,
}

fuzz_target!(|input: Input<'_>| {
    let handed_over = laid_out(&input);
    let (file, finished) = file_of(&handed_over);
    let cut_length = usize::from(input.cut_length).saturating_add(1);

    if !finished {
        assert_eq!(
            read_back(&file, cut_length).map_err(Error::kind),
            Err(ErrorKind::MissingMandatoryBox),
            "the bytes a refused writer laid down read as a file carrying a movie"
        );
        return;
    }

    let samples = handed_over.concat();
    let read = read_back(&file, cut_length)
        .unwrap_or_else(|failure| panic!("the reader rejects the file the writer laid down: {failure}"));
    assert_eq!(
        read, samples,
        "the samples were not read back as they were handed over"
    );

    let movie_first = movie_first_file_of(&handed_over);
    let moved = read_back(&movie_first, cut_length).unwrap_or_else(|failure| {
        panic!("the reader rejects the file with its movie moved first: {failure}")
    });
    assert_eq!(
        moved.iter().map(Sample::data).collect::<Vec<_>>(),
        samples.iter().map(Sample::data).collect::<Vec<_>>(),
        "the samples were not read back as they were handed over once the movie lay first"
    );
});

/// Movie of two tracks declaring no sample yet, the template the writer fills in
fn movie() -> MovieBox {
    let epoch = Mp4EpochSeconds::from_seconds(0);

    MovieBox::new(
        MovieHeaderBox::new(epoch, epoch, TIMESCALE, HeaderDuration::ZERO, NEXT_TRACK_ID),
        TRACK_IDS.map(track).to_vec(),
        None,
    )
    .expect("a movie of two tracks was refused")
}

/// The samples of `input`, chunk by chunk, as a caller hands them over
///
/// The decode time of a track follows the durations of the samples before it,
/// which is what the sample tables ask (§8.6.1.2), but for the one sample the
/// input breaks. The samples stop where the sample data runs out, so every one
/// of them carries a byte at least.
fn laid_out(input: &Input<'_>) -> Vec<Vec<Sample>> {
    let mut decode_times = [0u64; TRACK_IDS.len()];
    let mut taken: usize = 0;
    let mut samples_stated = 0u8;

    input
        .chunks
        .iter()
        .take(MAX_CHUNKS)
        .map(|chunk| {
            let position = usize::from(chunk.second_track);
            let track_id = TRACK_IDS.get(position).copied().unwrap_or(u32::MAX);
            chunk
                .samples
                .iter()
                .take(MAX_SAMPLES)
                .map_while(|stated| {
                    let end = taken
                        .saturating_add(usize::from(stated.length))
                        .saturating_add(1);
                    let data = input.sample_data.get(taken..end)?;
                    taken = end;

                    let broken = input.decode_time_broken_at == Some(samples_stated);
                    samples_stated = samples_stated.saturating_add(1);
                    let follows = decode_times.get(position).copied().unwrap_or(0);
                    let decode_time = if broken {
                        follows.saturating_add(1)
                    } else {
                        follows
                    };

                    if let Some(next) = decode_times.get_mut(position) {
                        *next = follows.saturating_add(u64::from(stated.duration));
                    }

                    Some(Sample::new(
                        track_id,
                        decode_time,
                        u32::from(stated.duration),
                        0,
                        SampleFlags::ZERO,
                        SAMPLE_DESCRIPTION_INDEX,
                        data.to_vec(),
                    ))
                })
                .collect()
        })
        .collect()
}

/// Lays the chunks down as a non-fragmented file, and reports whether the writer finished it
///
/// A writer that refuses reports that same failure for every call after it and
/// still hands over the bytes of the chunks it had laid down, which carry no
/// movie.
fn file_of(chunks: &[Vec<Sample>]) -> (Vec<u8>, bool) {
    let mut writer = NonFragmentedWriter::new();
    let mut file = Vec::new();
    let mut refused = None;

    writer
        .handle_file_type(file_type())
        .expect("a writer waiting for the brands refused them");
    writer
        .handle_movie(movie())
        .expect("a writer waiting for the movie refused it");

    'chunks: for samples in chunks {
        if let Err(reported) = writer.begin_chunk() {
            refused = Some(reported);
            break;
        }
        for sample in samples {
            if let Err(reported) = writer.handle_sample(sample.clone()) {
                refused = Some(reported);
                break 'chunks;
            }
        }
        drained_into(&mut writer, &mut file);
    }

    if refused.is_none() {
        refused = writer.finish().err();
    }
    drained_into(&mut writer, &mut file);

    match refused {
        Some(reported) => {
            assert_eq!(
                writer.handle_sample(a_sample()),
                Err(reported),
                "a refused writer took a sample instead of reporting its failure again"
            );
            assert_eq!(
                writer.finish(),
                Err(reported),
                "a refused writer reported another failure when the file was declared over"
            );

            (file, false)
        }
        None => {
            assert_eq!(
                writer.handle_sample(a_sample()).map_err(Error::kind),
                Err(ErrorKind::AlreadyFinished),
                "the writer took a sample after the file was declared over"
            );

            (file, true)
        }
    }
}

/// Takes what the writer has laid down into `file`
fn drained_into(writer: &mut NonFragmentedWriter, file: &mut Vec<u8>) {
    while let Some(written) = writer.poll_output() {
        file.extend_from_slice(&written);
    }
}

/// A sample of the first track, for the calls a refused or finished writer takes
fn a_sample() -> Sample {
    Sample::new(
        TRACK_IDS[0],
        0,
        1,
        0,
        SampleFlags::ZERO,
        SAMPLE_DESCRIPTION_INDEX,
        Vec::new(),
    )
}

/// The chunks carrying samples, laid down again with the movie before the media data by the fixture
///
/// The fixture lays one track out and takes no empty chunk, so what changes
/// is where the movie lies, not which track claims the bytes; the samples
/// read back are compared by their bytes alone.
fn movie_first_file_of(chunks: &[Vec<Sample>]) -> Vec<u8> {
    let chunks: Vec<Vec<&[u8]>> = chunks
        .iter()
        .filter(|samples| !samples.is_empty())
        .map(|samples| samples.iter().map(Sample::data).collect())
        .collect();
    let chunks: Vec<&[&[u8]]> = chunks.iter().map(Vec::as_slice).collect();

    non_fragmented_file(&chunks, true)
}

/// The samples `file` carries, read off it `cut_length` bytes at a time and then off the bytes it wants fetched
fn read_back(file: &[u8], cut_length: usize) -> Result<Vec<Sample>, Error> {
    let mut reader = NonFragmentedReader::new();
    let mut samples = Vec::new();

    for arriving in file.chunks(cut_length) {
        reader.handle_input(arriving)?;
        drain(&mut reader, &mut samples);
    }
    while let Some(wanted) = reader.wanted_extent() {
        let start = usize::try_from(wanted.start).unwrap_or(usize::MAX);
        let end = usize::try_from(wanted.end).unwrap_or(usize::MAX);
        let fetched = file.get(start..end).unwrap_or_default();

        reader.handle_data(wanted.start, fetched)?;
        drain(&mut reader, &mut samples);
        // Why not looping until nothing is wanted: a want past the file is
        // never met, and empty input leaves it standing
        if fetched.is_empty() {
            break;
        }
    }
    reader.finish()?;
    drain(&mut reader, &mut samples);

    Ok(samples)
}

/// Takes every sample the reader has completed
fn drain(reader: &mut NonFragmentedReader, samples: &mut Vec<Sample>) {
    while let Some(sample) = reader.poll_sample() {
        samples.push(sample);
    }
}
