//! Round-trip properties of [`FragmentedWriter`] against [`FragmentedReader`]
//!
//! One run lays samples down as a fragmented file, reads that file back, and
//! checks four properties of the same input:
//!
//! 1. no call panics: a writer that refuses a sample reports that same failure
//!    for every call after it, and still hands over the bytes it had laid down
//! 2. the samples of one track are read back as they were handed over
//! 3. the reader does not reject the file the writer laid down
//! 4. laying the samples read back out again reads back those same samples: the
//!    order the tracks arrive in is settled by the first pass, which gathers the
//!    samples of a track into one run, so the second pass is a fixed point of the
//!    whole presentation rather than of one track at a time
//!
//! The defaults the movie states are ones no fragment falls back on, so a sample
//! read back holding one of them would mean its fragment left the property to the
//! movie instead of stating it.

#![no_main]

use isobmff::{
    FileError, FileErrorKind, FragmentedReader, FragmentedWriter, MovieBox, Sample,
    TrackExtendsBox,
};
use isobmff_test_support::file_type;
use libfuzzer_sys::arbitrary::{self, Arbitrary};
use libfuzzer_sys::fuzz_target;

#[path = "helpers/movie.rs"]
mod movie;

use movie::{movie_of, track_id_of};

/// Tracks the samples of a run are laid out over
const TRACK_COUNT: usize = 2;

/// Fragments one run lays out at most
const MAX_FRAGMENTS: usize = 4;

/// Samples one fragment carries at most
const MAX_SAMPLES: usize = 8;

/// The `stsd` entry every sample of a run is described by
const SAMPLE_DESCRIPTION_INDEX: u32 = 1;

/// Input of one run: the samples of each fragment, and the bytes they are carried as
#[derive(Arbitrary, Debug)]
struct Input<'bytes> {
    /// Fragments laid out, in the order they are handed over
    fragments: Vec<Fragment>,
    /// Which sample, counted over the whole run, states a decode time of its own
    decode_time_broken_at: Option<u8>,
    /// Bytes the buffer laying the file down offers at a time
    buffer_length: u8,
    // Why not put `sample_data` first, and why a slice: only the last field is
    // handed what is left, and only `&[u8]` takes it verbatim — a `Vec<u8>` reads
    // a byte of its own before each element, so a seed stops at its first even
    // byte.
    sample_data: &'bytes [u8],
}

/// One fragment: the number it carries, and the samples handed over for it
#[derive(Arbitrary, Debug)]
struct Fragment {
    sequence_number: u32,
    samples: Vec<Stated>,
}

/// One sample as the caller states it, over the bytes it is carried as
#[derive(Arbitrary, Debug)]
struct Stated {
    /// Whether the sample belongs to the second of the tracks the movie declares
    second_track: bool,
    duration: u16,
    flags: u32,
    composition_time_offset: Option<i16>,
    /// Bytes of the sample data this sample takes
    length: u8,
}

fuzz_target!(|input: Input<'_>| {
    let Some(movie) = movie() else {
        return;
    };
    let buffer_length = usize::from(input.buffer_length).saturating_add(1);
    let handed_over = laid_out(&input);
    let (file, written_count) = file_of(&movie, &handed_over, buffer_length);
    let first_pass = read_back(&file);

    let taken = &handed_over[..written_count.min(handed_over.len())];
    let samples: Vec<Sample> = taken
        .iter()
        .flat_map(|(_sequence_number, samples)| samples.iter().cloned())
        .collect();

    for position in 0..TRACK_COUNT {
        let track_id = track_id_of(position);

        assert_eq!(
            of_track(&first_pass, track_id),
            of_track(&samples, track_id),
            "the samples of a track were not read back as they were handed over"
        );
    }
    assert_eq!(
        first_pass.len(),
        samples.len(),
        "the file reads back another number of samples than it was laid out from"
    );

    let again = regrouped(taken, &first_pass);
    let (file_again, _written_again) = file_of(&movie, &again, buffer_length);
    let read_back_again = read_back(&file_again);

    assert_eq!(
        first_pass, read_back_again,
        "laying the samples read back out again read back other samples"
    );
});

/// Movie of two fragmented tracks, stating defaults no fragment falls back on
fn movie() -> Option<MovieBox> {
    let never_fallen_back_on =
        |position| TrackExtendsBox::new(track_id_of(position), 9, 1, 1, u32::MAX);

    movie_of((0..TRACK_COUNT).map(never_fallen_back_on).collect())
}

/// The samples of `input`, fragment by fragment, as a caller hands them over
///
/// The decode time of a track follows the durations of the samples before it,
/// which is what a writer asks of a fragment (§8.8.12), but for the one sample
/// the input breaks.
fn laid_out(input: &Input<'_>) -> Vec<(u32, Vec<Sample>)> {
    let mut decode_times = [0u64; TRACK_COUNT];
    let mut taken: usize = 0;
    let mut samples_stated = 0u8;

    input
        .fragments
        .iter()
        .take(MAX_FRAGMENTS)
        .map(|fragment| {
            let samples = fragment
                .samples
                .iter()
                .take(MAX_SAMPLES)
                .map(|stated| {
                    let position = usize::from(stated.second_track);
                    let end = taken
                        .saturating_add(usize::from(stated.length))
                        .min(input.sample_data.len());
                    let data = input.sample_data.get(taken..end).unwrap_or_default();
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

                    Sample::new(
                        track_id_of(position),
                        decode_time,
                        u32::from(stated.duration),
                        stated.composition_time_offset.map(i64::from),
                        stated.flags,
                        SAMPLE_DESCRIPTION_INDEX,
                        data.to_vec(),
                    )
                })
                .collect();

            (fragment.sequence_number, samples)
        })
        .collect()
}

/// Lays the fragments down as a fragmented file, and reports how many were closed
///
/// A writer that refuses reports that same failure for every call after it and
/// still hands over the bytes of the fragments it had closed.
fn file_of(
    movie: &MovieBox,
    fragments: &[(u32, Vec<Sample>)],
    buffer_length: usize,
) -> (Vec<u8>, usize) {
    let mut writer = FragmentedWriter::new();
    let mut file = Vec::new();
    let mut closed = 0;
    let mut refused = None;

    writer
        .handle_file_type(file_type())
        .expect("a writer waiting for the brands refused them");
    writer
        .handle_movie(movie.clone())
        .expect("a writer waiting for the movie refused it");

    for (sequence_number, samples) in fragments {
        let mut outcome = writer.begin_fragment(*sequence_number);

        for sample in samples {
            if outcome.is_err() {
                break;
            }
            outcome = writer.handle_sample(sample.clone());
        }
        if outcome.is_ok() {
            outcome = writer.finish_fragment();
        }

        match outcome {
            Ok(()) => closed += 1,
            Err(reported) => {
                refused = Some(reported);
                break;
            }
        }
        drained_into(&mut writer, buffer_length, &mut file);
    }

    if refused.is_none() {
        refused = writer.finish().err();
    }
    drained_into(&mut writer, buffer_length, &mut file);

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
        }
        None => assert_eq!(
            writer.handle_sample(a_sample()).map_err(FileError::kind),
            Err(FileErrorKind::AlreadyFinished),
            "the writer took a sample after the file was declared over"
        ),
    }

    (file, closed)
}

/// Takes what the writer has laid down into `file`, a buffer at a time
fn drained_into(writer: &mut FragmentedWriter, buffer_length: usize, file: &mut Vec<u8>) {
    let mut buffer = vec![0; buffer_length];

    loop {
        let written = writer.poll_output(&mut buffer);

        match buffer.get(..written) {
            Some([]) | None => return,
            Some(bytes) => file.extend_from_slice(bytes),
        }
    }
}

/// A sample of the first track, for the calls a refused or finished writer takes
fn a_sample() -> Sample {
    Sample::new(
        track_id_of(0),
        0,
        1,
        None,
        0,
        SAMPLE_DESCRIPTION_INDEX,
        Vec::new(),
    )
}

/// The samples `file` carries, read back through the layout it was laid down as
///
/// Panics where the reader rejects the file, which is the property this target
/// holds the writer to.
fn read_back(file: &[u8]) -> Vec<Sample> {
    let mut reader = FragmentedReader::new();
    let mut samples = Vec::new();

    assert!(
        reader.handle_input(file).is_ok(),
        "the reader rejects the file the writer laid down"
    );
    while let Some(sample) = reader.poll_sample() {
        samples.push(sample);
    }

    assert!(
        reader.finish().is_ok(),
        "the reader rejects the end of the file the writer laid down"
    );
    while let Some(sample) = reader.poll_sample() {
        samples.push(sample);
    }

    samples
}

/// The samples read back, gathered into the fragments they were laid out as
fn regrouped(laid_out: &[(u32, Vec<Sample>)], read_back: &[Sample]) -> Vec<(u32, Vec<Sample>)> {
    let mut rest = read_back;

    laid_out
        .iter()
        .map(|(sequence_number, samples)| {
            let (taken, remainder) = rest.split_at(samples.len().min(rest.len()));
            rest = remainder;

            (*sequence_number, taken.to_vec())
        })
        .collect()
}

/// The samples of `track_id`, in the order they lie in `samples`
fn of_track(samples: &[Sample], track_id: u32) -> Vec<Sample> {
    samples
        .iter()
        .filter(|sample| sample.track_id() == track_id)
        .cloned()
        .collect()
}
