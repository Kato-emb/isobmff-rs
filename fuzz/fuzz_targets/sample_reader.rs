//! Reading properties of [`sample_extents`] feeding [`SampleReader`]
//!
//! One run lays out a movie, the fragments that continue it, and the media data
//! their samples claim, resolves each fragment against the movie once, and
//! hands the extents and the media data to one reader, checking seven
//! properties of the same input:
//!
//! 1. no call panics, a failure of the reader is reported again by every call
//!    after it, and a fragment the resolver refuses ends the presentation there
//! 2. how the media data is cut into the parts it arrives in does not change the
//!    samples read, and neither does handing every part over twice; where every
//!    fragment lies as the movie has it, the cut does not change their order
//!    either, the extents being held in the order of their bytes
//! 3. handing the parts over in reverse reads samples among those handing them
//!    over in order reads: a sample fills from its start, so bytes arriving
//!    before the ones they follow are passed over
//! 4. every sample read belongs to a track the movie declares, and no more
//!    samples are read than the fragments declared rows for
//! 5. where every fragment lies as the movie has it — anchored at the fragment or
//!    at the data before it, every run following the one before it — and the media
//!    data meets every claim, no sample is left short of its data, and the samples
//!    read are the samples declared, each carrying the bytes it was declared over
//! 6. where every fragment lies as the movie has it and the media data meets
//!    every claim, no fragment states a decode time of its own and none
//!    declares an empty duration, the samples of one track follow one another
//!    by their durations, the first of them at zero
//! 7. once the samples are declared over nothing more is taken, and the samples
//!    completed before that are still handed over
//!
//! What a fragment states about its samples is checked against what it declared,
//! never against the inheritance the resolver settles it through — the input
//! states each property in one place, which [`presentation`] lays out. That the
//! row of a run stands in front of the `tfhd` and the `tfhd` in front of the
//! `trex` (§8.8.7, §8.8.8) is the subject of the unit tests, which state the
//! layers apart.

#![no_main]

use std::collections::{BTreeMap, HashMap};

use isobmff::boxes::MovieBox;
use isobmff::sample::movie_fragment::sample_extents;
use isobmff::sample::{Error, ErrorKind, Sample, SampleExtent, SampleReader, TrackDecodeTimes};
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
    /// The extents a fragment declares, handed over together before the media data they lie in
    Extents(Vec<SampleExtent>),
    /// Media data that arrived, and where in the presentation it starts
    MediaData(u64, &'data [u8]),
}

/// Everything one pass over a presentation reported
#[derive(PartialEq, Debug)]
struct Reading {
    samples: Vec<Sample>,
    failure: Option<Error>,
}

fuzz_target!(|input: Input<'_>| {
    let Some(laid_out) = lay_out(&input) else {
        return;
    };
    let (extents, refused) = resolved(&laid_out);
    let limit = u64::from(input.sample_size_limit);
    let read_with = |lengths, arrival| {
        read(
            limit,
            steps(&laid_out, &extents, input.media_data, lengths, arrival),
            refused,
        )
    };

    let in_order = read_with(input.cut_lengths, Arrival::InOrder);
    let twice = read_with(input.cut_lengths, Arrival::Twice);
    let cut_smaller = read_with(SMALLEST_PARTS, Arrival::InOrder);
    let reversed = read_with(input.cut_lengths, Arrival::Reversed);

    assert_eq!(
        in_order, twice,
        "handing every part of the media data over twice changed the samples read"
    );
    if laid_out.lies_as_declared {
        assert_eq!(
            in_order, cut_smaller,
            "how the media data was cut changed the samples read, or their order"
        );
    } else {
        assert_eq!(
            in_order.failure, cut_smaller.failure,
            "how the media data was cut changed the failure reported"
        );
        assert_eq!(
            counted(&in_order.samples),
            counted(&cut_smaller.samples),
            "how the media data was cut changed the samples read"
        );
    }
    let whole = counted(&in_order.samples);
    assert!(
        counted(&reversed.samples)
            .iter()
            .all(|(sample, count)| whole.get(sample).is_some_and(|read| read >= count)),
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
            in_order.failure.map(Error::kind),
            Some(ErrorKind::UnfinishedSample),
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

    if laid_out.met_as_declared && follows_by_durations(&input) {
        samples_follow_by_their_durations(&in_order.samples);
    }
});

/// The extents each fragment declares, resolved against the movie in turn
///
/// A fragment the resolver refuses ends the presentation: the fragments before
/// it are what the reader is handed, and the refusal is reported with them.
fn resolved(laid_out: &LaidOut) -> (Vec<Vec<SampleExtent>>, Option<Error>) {
    let mut decode_times = TrackDecodeTimes::new();
    let mut extents = Vec::new();

    for placed in &laid_out.fragments {
        let of_fragment = sample_extents(
            &placed.movie_fragment,
            &laid_out.movie,
            placed.moof_start,
            &mut decode_times,
        )
        .and_then(|extents| extents.collect::<Result<Vec<_>, _>>());

        match of_fragment {
            Ok(of_fragment) => extents.push(of_fragment),
            Err(refused) => return (extents, Some(refused)),
        }
    }

    (extents, None)
}

/// The steps a caller takes over the presentation, its media data cut and ordered by `arrival`
fn steps<'data>(
    laid_out: &LaidOut,
    extents: &[Vec<SampleExtent>],
    media_data: &'data [u8],
    cut_lengths: [u8; 4],
    arrival: Arrival,
) -> Vec<Step<'data>> {
    let mut lengths = cut_lengths.into_iter().cycle();
    let mut steps = Vec::new();

    for (placed, of_fragment) in laid_out.fragments.iter().zip(extents) {
        steps.push(Step::Extents(of_fragment.clone()));

        let Some((mut start, held)) = placed.data.clone() else {
            continue;
        };
        let mut parts = Vec::new();
        let mut taken = held.start;

        while taken < held.end {
            let length = usize::from(lengths.next().unwrap_or(0)).saturating_add(1);
            let end = taken.saturating_add(length).min(held.end);
            let Some(part) = media_data.get(taken..end) else {
                break;
            };

            parts.push(Step::MediaData(start, part));
            start = start.saturating_add(u64::try_from(part.len()).unwrap_or(0));
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
/// Where the resolver refused a fragment, the presentation ends there and the
/// samples are never declared over.
fn read(sample_size_limit: u64, steps: Vec<Step<'_>>, refused: Option<Error>) -> Reading {
    let mut reader = SampleReader::with_sample_size_limit(sample_size_limit);
    let mut samples = Vec::new();
    let mut failure = None;

    for step in steps {
        let outcome = match step {
            Step::Extents(extents) => reader.handle_sample_extents(extents.into_iter().map(Ok)),
            Step::MediaData(offset, data) => reader.handle_data(offset, data),
        };

        drain(&mut reader, &mut samples);

        if let Err(reported) = outcome {
            assert_eq!(
                reader.handle_data(0, &[]),
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

    if failure.is_none() && refused.is_none() {
        let over = reader.finish();

        drain(&mut reader, &mut samples);

        match over {
            Ok(()) => assert_eq!(
                reader.handle_data(0, &[]).map_err(Error::kind),
                Err(ErrorKind::AlreadyFinished),
                "the reader took media data after the samples were declared over"
            ),
            Err(reported) => failure = Some(reported),
        }
    }

    Reading {
        samples,
        failure: failure.or(refused),
    }
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

/// The samples by how many times each was read, whatever order they came out in
fn counted(samples: &[Sample]) -> HashMap<&Sample, usize> {
    let mut counted = HashMap::new();

    for sample in samples {
        *counted.entry(sample).or_insert(0) += 1;
    }

    counted
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
