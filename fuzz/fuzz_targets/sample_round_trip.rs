//! Round-trip properties of [`SampleWriter`] against [`SampleReader`]
//!
//! One run lays samples out as movie fragments, reads back the file they make,
//! and checks four properties of the same input:
//!
//! 1. no call panics: a writer that refuses a sample reports that same failure
//!    for every call after it, and still hands over the fragments it had closed
//! 2. the samples of one track are read back as they were handed over
//! 3. neither layer rejects the file the writer laid down
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
    BoxDefinition, BoxHeader, MediaDataBox, MovieBox, MovieFragmentBox, Sample, SampleError,
    SampleErrorKind, SampleReader, SampleWriter, TrackExtendsBox,
};
use isobmff_sequence::{BoxEvent, BoxReader};
use isobmff_test_support::{bytes_of, file_type};
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
    let written = write(&handed_over);
    let written_count = written.len();

    let Some(file) = file_of(&movie, written, buffer_length) else {
        return;
    };
    let Some(first_pass) = read_back(&movie, &file) else {
        return;
    };

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
    let laid_out_again = write(&again);
    let Some(file_again) = file_of(&movie, laid_out_again, buffer_length) else {
        return;
    };
    let Some(read_back_again) = read_back(&movie, &file_again) else {
        return;
    };

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

/// Hands the fragments to a writer and gathers the pairs it laid them out as
fn write(fragments: &[(u32, Vec<Sample>)]) -> Vec<(MovieFragmentBox, MediaDataBox)> {
    let mut writer = SampleWriter::new();
    let mut closed = 0;
    let mut refused = None;

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
    }

    if refused.is_none() {
        refused = writer.finish().err();
    }

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
                "a refused writer reported another failure when the fragments were declared over"
            );
        }
        None => assert_eq!(
            writer.handle_sample(a_sample()).map_err(SampleError::kind),
            Err(SampleErrorKind::AlreadyFinished),
            "the writer took a sample after the fragments were declared over"
        ),
    }

    let mut laid_down = Vec::new();
    while let Some(pair) = writer.poll_fragment() {
        laid_down.push(pair);
    }

    assert_eq!(
        laid_down.len(),
        closed,
        "the writer handed over another number of fragments than it closed"
    );

    laid_down
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

/// The file the pairs make: the brands, the movie, then fragment after fragment
fn file_of(
    movie: &MovieBox,
    laid_down: Vec<(MovieFragmentBox, MediaDataBox)>,
    buffer_length: usize,
) -> Option<Vec<u8>> {
    let mut events = vec![
        BoxEvent::FileType(file_type()),
        BoxEvent::Movie(movie.clone()),
    ];

    for (movie_fragment, media_data) in laid_down {
        let payload_len = u64::try_from(media_data.data().len()).ok()?;
        let header = BoxHeader::with_payload_len(MediaDataBox::BOX_TYPE, payload_len)?;

        events.push(BoxEvent::MovieFragment(movie_fragment));
        events.push(BoxEvent::RawStart(header));
        if !media_data.data().is_empty() {
            events.push(BoxEvent::RawPayload(media_data.into_data()));
        }
        events.push(BoxEvent::RawEnd);
    }

    bytes_of(events, buffer_length).ok()
}

/// The samples `file` carries, read through the box layer beside this one
///
/// Reports `None` where the movie continues in no fragments, and panics where
/// either layer rejects the file — which is the property this target holds the
/// writer to.
fn read_back(movie: &MovieBox, file: &[u8]) -> Option<Vec<Sample>> {
    let mut box_reader = BoxReader::new();
    let mut sample_reader = SampleReader::new(movie).ok()?;
    let mut samples = Vec::new();

    assert!(
        box_reader.handle_input(file).is_ok(),
        "the box layer rejects the file the writer laid down"
    );

    while let Some(event) = box_reader.poll_event() {
        let extent = box_reader
            .event_extent()
            .expect("an event was taken, so it has an extent");
        let taken = match event {
            BoxEvent::MovieFragment(movie_fragment) => {
                sample_reader.handle_movie_fragment(movie_fragment, extent)
            }
            BoxEvent::RawPayload(payload) => sample_reader.handle_media_data(&payload, extent),
            _passed_over => Ok(()),
        };

        assert!(
            taken.is_ok(),
            "the sample layer rejects a fragment the writer laid out"
        );
        while let Some(sample) = sample_reader.poll_sample() {
            samples.push(sample);
        }
    }

    assert!(
        box_reader.finish().is_ok(),
        "the box layer rejects the end of the file the writer laid down"
    );
    assert!(
        sample_reader.finish().is_ok(),
        "the sample layer is left short of the data the file laid down"
    );

    Some(samples)
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
