//! Reading properties of [`SampleReader`]
//!
//! One run lays out a movie, the fragments that continue it, and the media data
//! their samples claim, and checks seven properties of the same input:
//!
//! 1. no call panics, and a failure is reported again by every call after it
//! 2. how the media data is cut into the parts it arrives in does not change the
//!    samples read, and neither does handing every part over twice
//! 3. handing the parts over in reverse reads a prefix of what handing them over
//!    in order reads: a sample fills from its start, so bytes arriving before the
//!    ones they follow are passed over
//! 4. every sample read belongs to a track the movie declares, and no more
//!    samples are read than the fragments declared rows for
//! 5. where every fragment lies as the movie has it — anchored at the fragment or
//!    at the data before it, every run following the one before it — and the media
//!    data meets every claim, no sample is left short of its data, and the samples
//!    read are the samples declared, each carrying the bytes it was declared over
//! 6. where no fragment states a decode time of its own and none declares an
//!    empty duration, the samples of one track follow one another by their
//!    durations, the first of them at zero
//! 7. once the samples are declared over nothing more is taken, and the samples
//!    completed before that are still handed over
//!
//! What a fragment states about its samples is checked against what it declared,
//! never against the inheritance the reader resolves it through — the input states
//! each property in one place, which [`presentation`] lays out. That the row of a
//! run stands in front of the `tfhd` and the `tfhd` in front of the `trex`
//! (§8.8.7, §8.8.8) is the subject of the unit tests, which state the layers apart.

#![no_main]

use std::collections::BTreeMap;
use std::ops::Range;

use isobmff::{MovieBox, MovieFragmentBox, Sample, SampleError, SampleErrorKind, SampleReader};
use libfuzzer_sys::fuzz_target;

#[path = "sample_reader/presentation.rs"]
mod presentation;

use presentation::{Input, LaidOut, follows_by_durations, lay_out};

/// Lengths the media data is cut into where a run cuts it as small as it goes
const SMALLEST_PARTS: [u8; 4] = [0; 4];

/// How the media data of a fragment reaches the reader
#[derive(Clone, Copy)]
enum Arrival {
    /// Each part once, in the order the samples claim them
    InOrder,
    /// Each part twice over
    Twice,
    /// Each part once, the last of them first
    Reversed,
}

/// What a caller hands the reader next
#[derive(Clone)]
enum Step<'data> {
    /// A movie fragment, and the bytes of the presentation it occupies
    Fragment(MovieFragmentBox, Range<u64>),
    /// Media data that arrived, and the bytes of the presentation it holds
    MediaData(Range<u64>, &'data [u8]),
}

/// Everything one pass of the reader over a presentation reported
#[derive(PartialEq, Debug)]
struct Reading {
    samples: Vec<Sample>,
    failure: Option<SampleError>,
}

fuzz_target!(|input: Input<'_>| {
    let Some(laid_out) = lay_out(&input) else {
        return;
    };
    let limit = u64::from(input.sample_size_limit);
    let read_with = |lengths, arrival| {
        read(
            &laid_out.movie,
            limit,
            steps(&laid_out, input.media_data, lengths, arrival),
        )
    };

    let Some(in_order) = read_with(input.cut_lengths, Arrival::InOrder) else {
        return;
    };
    let Some(twice) = read_with(input.cut_lengths, Arrival::Twice) else {
        return;
    };
    let Some(cut_smaller) = read_with(SMALLEST_PARTS, Arrival::InOrder) else {
        return;
    };
    let Some(reversed) = read_with(input.cut_lengths, Arrival::Reversed) else {
        return;
    };

    assert_eq!(
        in_order, twice,
        "handing every part of the media data over twice changed the samples read"
    );
    assert_eq!(
        in_order, cut_smaller,
        "how the media data was cut changed the samples read"
    );
    assert!(
        in_order.samples.starts_with(&reversed.samples),
        "media data arriving in reverse read samples the whole of it does not"
    );

    for sample in &in_order.samples {
        assert!(
            declares(&laid_out.movie, sample.track_id()),
            "a sample of a track the movie never declared was read"
        );
    }
    assert!(
        in_order.samples.len() <= laid_out.rows,
        "more samples were read than the fragments declared rows for"
    );

    if laid_out.met_as_declared {
        assert_ne!(
            in_order.failure.map(SampleError::kind),
            Some(SampleErrorKind::UnfinishedSample),
            "a sample was left short of data every claim of it was met by"
        );

        if in_order.failure.is_none() {
            assert_eq!(
                reported(&in_order.samples),
                laid_out.declared_as(input.media_data),
                "the samples read are not the samples the fragments declared"
            );
        }
    }

    if follows_by_durations(&input) {
        samples_follow_by_their_durations(&in_order.samples);
    }
});

/// The steps a caller takes over the presentation, its media data cut and ordered by `arrival`
fn steps<'data>(
    laid_out: &LaidOut,
    media_data: &'data [u8],
    cut_lengths: [u8; 4],
    arrival: Arrival,
) -> Vec<Step<'data>> {
    let mut lengths = cut_lengths.into_iter().cycle();
    let mut steps = Vec::new();

    for placed in &laid_out.fragments {
        steps.push(Step::Fragment(
            placed.movie_fragment.clone(),
            placed.extent.clone(),
        ));

        let Some((extent, held)) = placed.data.clone() else {
            continue;
        };
        let mut parts = Vec::new();
        let mut start = extent.start;
        let mut taken = held.start;

        while taken < held.end {
            let length = usize::from(lengths.next().unwrap_or(0)).saturating_add(1);
            let end = taken.saturating_add(length).min(held.end);
            let Some(part) = media_data.get(taken..end) else {
                break;
            };
            let covers = u64::try_from(end.saturating_sub(taken)).unwrap_or(0);

            parts.push(Step::MediaData(start..start.saturating_add(covers), part));
            start = start.saturating_add(covers);
            taken = end;
        }

        match arrival {
            Arrival::InOrder => steps.extend(parts),
            Arrival::Twice => steps.extend(parts.into_iter().flat_map(|part| [part.clone(), part])),
            Arrival::Reversed => steps.extend(parts.into_iter().rev()),
        }
    }

    steps
}

/// Hands the steps of a presentation to a reader and gathers what it reports
///
/// Reports `None` where the movie continues in no fragments at all, which leaves
/// the reader nothing to build from.
fn read(movie: &MovieBox, sample_size_limit: u64, steps: Vec<Step<'_>>) -> Option<Reading> {
    let mut reader = SampleReader::with_sample_size_limit(movie, sample_size_limit).ok()?;
    let mut samples = Vec::new();
    let mut failure = None;

    for step in steps {
        let outcome = match step {
            Step::Fragment(movie_fragment, extent) => {
                reader.handle_movie_fragment(movie_fragment, extent)
            }
            Step::MediaData(extent, data) => reader.handle_media_data(data, extent),
        };

        drain(&mut reader, &mut samples);

        if let Err(reported) = outcome {
            assert_eq!(
                reader.handle_media_data(&[], 0..0),
                Err(reported),
                "a failed reader took media data instead of reporting its failure again"
            );
            assert_eq!(
                reader.finish(),
                Err(reported),
                "a failed reader reported another failure when the samples were declared over"
            );
            failure = Some(reported);
            break;
        }
    }

    if failure.is_none() {
        let over = reader.finish();

        drain(&mut reader, &mut samples);

        match over {
            Ok(()) => assert_eq!(
                reader
                    .handle_media_data(&[], 0..0)
                    .map_err(SampleError::kind),
                Err(SampleErrorKind::AlreadyFinished),
                "the reader took media data after the samples were declared over"
            ),
            Err(reported) => failure = Some(reported),
        }
    }

    Some(Reading { samples, failure })
}

/// Takes every sample the reader has completed
fn drain(reader: &mut SampleReader, samples: &mut Vec<Sample>) {
    while let Some(sample) = reader.poll_sample() {
        samples.push(sample);
    }
}

/// Returns whether `track_id` names a track of `movie`
fn declares(movie: &MovieBox, track_id: u32) -> bool {
    movie
        .trak()
        .iter()
        .any(|trak| trak.tkhd().track_id() == track_id)
}

/// The samples read, by the track each belongs to and the bytes it carries
fn reported(samples: &[Sample]) -> Vec<(u32, Vec<u8>)> {
    samples
        .iter()
        .map(|sample| (sample.track_id(), sample.data().to_vec()))
        .collect()
}

/// Checks that the samples of every track follow one another by their durations
fn samples_follow_by_their_durations(samples: &[Sample]) {
    let mut next_of_track: BTreeMap<u32, u64> = BTreeMap::new();

    for sample in samples {
        let decoded_at = next_of_track.get(&sample.track_id()).copied().unwrap_or(0);

        assert_eq!(
            sample.decode_time(),
            decoded_at,
            "a sample does not follow the one before it on the timeline of its track"
        );
        next_of_track.insert(
            sample.track_id(),
            decoded_at.saturating_add(u64::from(sample.sample_duration())),
        );
    }
}
