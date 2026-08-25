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
//! never against the inheritance the reader resolves it through: the run states
//! the properties of its samples, or its `tfhd` does, or the `trex` of its track
//! does, and one input states them in one place. That the row of a run stands in
//! front of the `tfhd` and the `tfhd` in front of the `trex` (§8.8.7, §8.8.8) is
//! the subject of the unit tests, which state the layers apart.

#![no_main]

use std::collections::BTreeMap;
use std::ops::Range;

use isobmff::{
    FullBoxFlags, MovieBox, MovieFragmentBox, MovieFragmentHeaderBox, Sample, SampleError,
    SampleErrorKind, SampleReader, TrackExtendsBox, TrackFragmentBaseMediaDecodeTimeBox,
    TrackFragmentBox, TrackFragmentHeaderBox, TrackRunBox, TrackRunSample,
};
use libfuzzer_sys::arbitrary::{self, Arbitrary};
use libfuzzer_sys::fuzz_target;

#[path = "helpers/movie.rs"]
mod movie;

use movie::{movie_of, track_id_of};

/// Tracks the movie of a run declares
const TRACK_COUNT: usize = 2;

/// Movie fragments one run hands over at most
const MAX_FRAGMENTS: usize = 3;

/// Track fragments one movie fragment carries at most
const MAX_TRACK_FRAGMENTS: usize = 3;

/// Track runs one track fragment carries at most
const MAX_TRACK_RUNS: usize = 3;

/// Rows one track run carries at most
const MAX_ROWS: usize = 8;

/// Bytes the header of a movie fragment takes
const HEADER_LEN: u64 = 8;

/// Lengths the media data is cut into where a run cuts it as small as it goes
const SMALLEST_PARTS: [u8; 4] = [0; 4];

/// Input of one run: the movie, the fragments continuing it, and the media data
#[derive(Arbitrary, Debug)]
struct Input<'bytes> {
    /// Defaults each track of the movie states in its `trex`
    track_defaults: [TrackDefaults; TRACK_COUNT],
    /// Bytes one sample may declare
    sample_size_limit: u8,
    /// Fragments handed over, in the order they arrive
    fragments: Vec<Fragment>,
    /// Which track fragment, counted over the whole run, states a track no movie declares
    undeclared_track_at: Option<u8>,
    /// Lengths the media data is cut into, cycled
    cut_lengths: [u8; 4],
    // Why not put `media_data` first, and why a slice: only the last field is
    // handed what is left, and only `&[u8]` takes it verbatim — a `Vec<u8>`
    // reads a byte of its own before each element, so a seed stops at its first
    // even byte.
    media_data: &'bytes [u8],
}

/// Defaults the `trex` of one track states for the fragments that follow
#[derive(Arbitrary, Debug)]
struct TrackDefaults {
    sample_description_index: u32,
    sample_duration: u16,
    sample_size: u8,
    sample_flags: u32,
}

/// One movie fragment: where it lies, and the track fragments it carries
#[derive(Arbitrary, Debug)]
struct Fragment {
    sequence_number: u32,
    /// Bytes the movie fragment occupies, past the header of a box
    length: u8,
    /// Bytes between the fragment and the data its samples claim
    gap: u8,
    track_fragments: Vec<TrackFragment>,
}

/// One track fragment: the track it carries, where its data lies, and its runs
#[derive(Arbitrary, Debug)]
struct TrackFragment {
    /// Whether the fragment carries the second of the tracks the movie declares
    second_track: bool,
    anchor: Anchor,
    /// Whether the fragment declares an empty duration, and so carries no run
    duration_is_empty: bool,
    base_media_decode_time: Option<u64>,
    sample_description_index: Option<u32>,
    stated_at: StatedAt,
    runs: Vec<TrackRun>,
}

/// What the offsets of a track fragment are measured from (§8.8.7.1)
#[derive(Arbitrary, Debug)]
enum Anchor {
    /// The offset the fragment states, wherever that lands
    Stated(u64),
    /// The movie fragment itself
    MovieFragment,
    /// Where the data of the track fragment before it ended
    Unstated,
}

/// Where the properties a sample takes are stated
#[derive(Arbitrary, Clone, Copy, Debug)]
enum StatedAt {
    /// The row of the run the sample lies in
    Row,
    /// The header of the track fragment, for every sample of it alike
    TrackFragment,
    /// The extends box of the track, for every fragment of it alike
    Track,
}

/// One track run: where its data lies, and the rows it carries
#[derive(Arbitrary, Debug)]
struct TrackRun {
    offset: TrackRunOffset,
    /// Flags the first sample takes, where the rows of the track run state none
    first_sample_flags: Option<u32>,
    /// Whether the rows state an offset from their decode time to their composition time
    states_composition_time_offset: bool,
    rows: Vec<Row>,
}

/// Where the data of a track run lies (§8.8.8)
#[derive(Arbitrary, Debug)]
enum TrackRunOffset {
    /// Where the data the run claims was laid down
    AtTheData,
    /// Where the run before it ended
    Unstated,
    /// The offset the run states, wherever that lands
    Stated(i16),
}

/// One row of a track run: what it states about one sample
#[derive(Arbitrary, Debug)]
struct Row {
    size: u8,
    duration: u16,
    flags: u32,
    composition_time_offset: i16,
}

/// One movie fragment as it lies in the presentation
struct Placed {
    movie_fragment: MovieFragmentBox,
    /// Bytes of the presentation the `moof` occupies
    extent: Range<u64>,
    /// Where the data its samples claim lies, and the media data that meets it
    data: Option<(Range<u64>, Range<usize>)>,
}

/// One sample as the fragments declared it
struct Declared {
    track_id: u32,
    /// Bytes of the media data the sample is carried as
    data: Range<usize>,
}

/// A presentation laid out from one input
struct LaidOut {
    movie: MovieBox,
    fragments: Vec<Placed>,
    /// The samples the fragments declared, in the order they declared them
    declared: Vec<Declared>,
    /// Rows the fragments carry, however they lie
    rows: usize,
    /// Whether every fragment lies as the movie has it, every claim met
    met_as_declared: bool,
}

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
                declared_as(&laid_out, input.media_data),
                "the samples read are not the samples the fragments declared"
            );
        }
    }

    if follows_by_durations(&input) {
        samples_follow_by_their_durations(&in_order.samples);
    }
});

/// Lays the movie, the fragments and the media data of `input` out over a presentation
///
/// Reports `None` where the boxes of the input do not build: a fragment stating
/// what a box cannot carry is an input this run passes over rather than a failure
/// of the reader.
fn lay_out(input: &Input<'_>) -> Option<LaidOut> {
    let trex = input
        .track_defaults
        .iter()
        .enumerate()
        .map(|(position, defaults)| {
            TrackExtendsBox::new(
                track_id_of(position),
                defaults.sample_description_index,
                u32::from(defaults.sample_duration),
                u32::from(defaults.sample_size),
                defaults.sample_flags,
            )
        })
        .collect();
    let movie = movie_of(trex)?;

    let mut fragments = Vec::new();
    let mut declared = Vec::new();
    let mut rows = 0;
    let mut met_as_declared = true;
    let mut cursor: u64 = 0;
    let mut media_data_taken: usize = 0;
    let mut track_fragments_laid: u8 = 0;

    for fragment in input.fragments.iter().take(MAX_FRAGMENTS) {
        let moof_start = cursor;
        let moof_end = moof_start
            .checked_add(u64::from(fragment.length))?
            .checked_add(HEADER_LEN)?;
        let claim_start = moof_end.checked_add(u64::from(fragment.gap))?;

        let mut track_fragments = Vec::new();
        let mut fragment_declared: Vec<(u32, u32)> = Vec::new();
        let mut lies_as_declared = true;
        let mut data_cursor = claim_start;
        let mut data_before = None;

        for track_fragment in fragment.track_fragments.iter().take(MAX_TRACK_FRAGMENTS) {
            let laid_at = track_fragments_laid;
            track_fragments_laid = track_fragments_laid.saturating_add(1);

            let track_id = if input.undeclared_track_at == Some(laid_at) {
                lies_as_declared = false;
                track_id_of(TRACK_COUNT)
            } else if track_fragment.second_track {
                track_id_of(1)
            } else {
                track_id_of(0)
            };
            let (base_data_offset, anchor) = match track_fragment.anchor {
                Anchor::Stated(stated) => {
                    lies_as_declared = false;
                    (Some(stated), stated)
                }
                Anchor::MovieFragment => (None, moof_start),
                Anchor::Unstated => (None, data_before.unwrap_or(moof_start)),
            };
            let first_row = track_fragment
                .runs
                .first()
                .and_then(|track_run| track_run.rows.first());
            let states_defaults = matches!(track_fragment.stated_at, StatedAt::TrackFragment);
            let flags = if track_fragment.duration_is_empty {
                TrackFragmentHeaderBox::DURATION_IS_EMPTY
            } else if matches!(track_fragment.anchor, Anchor::MovieFragment) {
                TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF
            } else {
                FullBoxFlags::ZERO
            };
            let tfhd = TrackFragmentHeaderBox::new(
                flags,
                track_id,
                base_data_offset,
                track_fragment.sample_description_index,
                states_defaults.then(|| first_row.map_or(0, |row| u32::from(row.duration))),
                states_defaults.then(|| first_row.map_or(0, |row| u32::from(row.size))),
                states_defaults.then(|| first_row.map_or(0, |row| row.flags)),
            )?;
            let declared_size_of = |row: &Row| match track_fragment.stated_at {
                StatedAt::Row => u32::from(row.size),
                StatedAt::TrackFragment => first_row.map_or(0, |row| u32::from(row.size)),
                StatedAt::Track => track_size_of(input, track_id),
            };

            let mut runs = Vec::new();
            let carried = if track_fragment.duration_is_empty {
                &[][..]
            } else {
                &track_fragment.runs
            };

            for track_run in carried.iter().take(MAX_TRACK_RUNS) {
                let mut samples = Vec::new();
                let mut sizes = Vec::new();

                for row in track_run.rows.iter().take(MAX_ROWS) {
                    let stated_by_the_row = matches!(track_fragment.stated_at, StatedAt::Row);
                    let sample = TrackRunSample::new(
                        stated_by_the_row.then(|| u32::from(row.duration)),
                        stated_by_the_row.then(|| u32::from(row.size)),
                        stated_by_the_row.then_some(row.flags),
                        track_run
                            .states_composition_time_offset
                            .then(|| i64::from(row.composition_time_offset)),
                    )?;

                    samples.push(sample);
                    sizes.push(declared_size_of(row));
                }

                let data_offset = match track_run.offset {
                    TrackRunOffset::AtTheData => {
                        let past_the_anchor = data_cursor
                            .checked_sub(anchor)
                            .and_then(|distance| i32::try_from(distance).ok());

                        if past_the_anchor.is_none() {
                            lies_as_declared = false;
                        }
                        Some(past_the_anchor.unwrap_or(0))
                    }
                    TrackRunOffset::Unstated => {
                        // Why not laying the data of a first run out as declared:
                        // it begins at the anchor of its fragment, which a
                        // fragment anchored at itself puts back over the `moof`.
                        if runs.is_empty() {
                            lies_as_declared = false;
                        }
                        None
                    }
                    TrackRunOffset::Stated(stated) => {
                        lies_as_declared = false;
                        Some(i32::from(stated))
                    }
                };
                let first_sample_flags = match track_fragment.stated_at {
                    StatedAt::Row => None,
                    StatedAt::TrackFragment | StatedAt::Track => track_run.first_sample_flags,
                };

                runs.push(TrackRunBox::new(data_offset, first_sample_flags, samples)?);

                for size in sizes {
                    fragment_declared.push((track_id, size));
                    data_cursor = data_cursor.checked_add(u64::from(size))?;
                }
            }

            let tfdt = track_fragment
                .base_media_decode_time
                .map(TrackFragmentBaseMediaDecodeTimeBox::new);

            track_fragments.push(TrackFragmentBox::new(tfhd, tfdt, runs)?);
            data_before = Some(data_cursor);
        }

        rows += fragment_declared.len();
        let claimed = usize::try_from(data_cursor.checked_sub(claim_start)?).ok()?;
        let data = if lies_as_declared {
            let laid_down = media_data_taken
                .saturating_add(claimed)
                .min(input.media_data.len());
            let arrived = laid_down.saturating_sub(media_data_taken);

            if arrived < claimed {
                met_as_declared = false;
            }
            for (track_id, size) in fragment_declared {
                let end = media_data_taken.saturating_add(usize::try_from(size).ok()?);

                declared.push(Declared {
                    track_id,
                    data: media_data_taken..end,
                });
                media_data_taken = end;
            }
            media_data_taken = laid_down;

            Some((
                claim_start..claim_start.saturating_add(u64::try_from(arrived).ok()?),
                laid_down.saturating_sub(arrived)..laid_down,
            ))
        } else {
            met_as_declared = false;
            None
        };

        fragments.push(Placed {
            movie_fragment: MovieFragmentBox::new(
                MovieFragmentHeaderBox::new(fragment.sequence_number),
                track_fragments,
            ),
            extent: moof_start..moof_end,
            data,
        });
        cursor = data_cursor;
    }

    Some(LaidOut {
        movie,
        fragments,
        declared,
        rows,
        met_as_declared,
    })
}

/// The size the `trex` of `track_id` states, for the fragments that state none
fn track_size_of(input: &Input<'_>, track_id: u32) -> u32 {
    let position = usize::try_from(track_id.saturating_sub(1)).unwrap_or(TRACK_COUNT);

    input
        .track_defaults
        .get(position)
        .map_or(0, |defaults| u32::from(defaults.sample_size))
}

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

/// The samples the fragments declared, by the bytes of the media data laid down for each
fn declared_as(laid_out: &LaidOut, media_data: &[u8]) -> Vec<(u32, Vec<u8>)> {
    laid_out
        .declared
        .iter()
        .map(|declared| {
            let carried = media_data.get(declared.data.clone()).unwrap_or_default();

            (declared.track_id, carried.to_vec())
        })
        .collect()
}

/// Returns whether every fragment leaves the decode time of its track to the durations before it
fn follows_by_durations(input: &Input<'_>) -> bool {
    input
        .fragments
        .iter()
        .take(MAX_FRAGMENTS)
        .flat_map(|fragment| fragment.track_fragments.iter().take(MAX_TRACK_FRAGMENTS))
        .all(|track_fragment| {
            track_fragment.base_media_decode_time.is_none() && !track_fragment.duration_is_empty
        })
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
