//! Laying an input out as the presentation a reader is handed
//!
//! One input describes the movie a presentation continues in fragments, the
//! fragments themselves, and where the data their samples claim lies. What a
//! fragment states about its samples is stated in one place — the row of a track
//! run, the `tfhd` of the fragment, or the `trex` of the track — so what the
//! samples were declared as is known without resolving the inheritance the
//! resolver settles them through.
//!
//! Reached from the target with `#[path = "sample_reader/presentation.rs"] mod
//! presentation;`. A file under `fuzz_targets/` is a target only where the
//! `[[bin]]` table names it, so this module is not one.

use std::ops::Range;

use isobmff::boxes::{
    CompositionTimeOffset, MovieBox, MovieFragmentBox, MovieFragmentHeaderBox, SampleFlags,
    TrackExtendsBox, TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentBox, TrackFragmentHeaderBox,
    TrackFragmentHeaderFlags, TrackRunBox, TrackRunSample,
};
use libfuzzer_sys::arbitrary::{self, Arbitrary, Unstructured};

#[path = "../helpers/movie.rs"]
mod movie;
#[path = "../helpers/sample_flags.rs"]
mod sample_flags;

use movie::{SAMPLE_DESCRIPTION_INDEX, movie_of, track_id_of};
use sample_flags::sample_flags;

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

/// Input of one run: the movie, the fragments continuing it, and the media data
#[derive(Arbitrary, Debug)]
pub struct Input<'bytes> {
    /// Defaults each track of the movie states in its `trex`
    track_defaults: [TrackDefaults; TRACK_COUNT],
    /// Bytes one sample may declare
    pub sample_size_limit: u8,
    /// Fragments handed over, in the order they arrive
    fragments: Vec<Fragment>,
    /// Which track fragment, counted over the whole run, states a track no movie declares
    undeclared_track_at: Option<u8>,
    /// Lengths the media data is cut into, cycled
    pub cut_lengths: [u8; 4],
    // Why not put `media_data` first, and why a slice: only the last field is
    // handed what is left, and only `&[u8]` takes it verbatim — a `Vec<u8>`
    // reads a byte of its own before each element, so a seed stops at its first
    // even byte.
    pub media_data: &'bytes [u8],
}

/// Defaults the `trex` of one track states for the fragments that follow
#[derive(Arbitrary, Debug)]
pub struct TrackDefaults {
    sample_duration: u16,
    sample_size: u8,
    #[arbitrary(with = sample_flags)]
    sample_flags: SampleFlags,
}

/// One movie fragment: where it lies, and the track fragments it carries
#[derive(Arbitrary, Debug)]
pub struct Fragment {
    sequence_number: u32,
    /// Bytes the movie fragment occupies, past the header of a box
    length: u8,
    /// Bytes between the fragment and the data its samples claim
    gap: u8,
    track_fragments: Vec<TrackFragment>,
}

/// One track fragment: the track it carries, where its data lies, and its runs
#[derive(Arbitrary, Debug)]
pub struct TrackFragment {
    /// Whether the fragment carries the second of the tracks the movie declares
    second_track: bool,
    anchor: Anchor,
    /// Whether the fragment declares an empty duration, and so carries no run
    duration_is_empty: bool,
    base_media_decode_time: Option<u64>,
    /// Whether the fragment states the `stsd` entry itself rather than leaving it to the `trex`
    states_sample_description_index: bool,
    stated_at: StatedAt,
    runs: Vec<TrackRun>,
}

/// What the offsets of a track fragment are measured from (§8.8.7.1)
#[derive(Arbitrary, Debug)]
pub enum Anchor {
    /// The offset the fragment states, wherever that lands
    Stated(u64),
    /// The movie fragment itself
    MovieFragment,
    /// Where the data of the track fragment before it ended
    Unstated,
}

/// Where the properties a sample takes are stated
#[derive(Arbitrary, Clone, Copy, Debug)]
pub enum StatedAt {
    /// The row of the run the sample lies in
    Row,
    /// The header of the track fragment, for every sample of it alike
    TrackFragment,
    /// The extends box of the track, for every fragment of it alike
    Track,
}

/// One track run: where its data lies, and the rows it carries
#[derive(Arbitrary, Debug)]
pub struct TrackRun {
    offset: TrackRunOffset,
    /// Flags the first sample takes, where the rows of the track run state none
    #[arbitrary(with = first_sample_flags)]
    first_sample_flags: Option<SampleFlags>,
    /// Whether the rows state an offset from their decode time to their composition time
    states_composition_time_offset: bool,
    rows: Vec<Row>,
}

/// Where the data of a track run lies (§8.8.8)
#[derive(Arbitrary, Debug)]
pub enum TrackRunOffset {
    /// Where the data the run claims was laid down
    AtTheData,
    /// Where the run before it ended
    Unstated,
    /// The offset the run states, wherever that lands
    Stated(i16),
}

/// One row of a track run: what it states about one sample
#[derive(Arbitrary, Debug)]
pub struct Row {
    size: u8,
    duration: u16,
    #[arbitrary(with = sample_flags)]
    flags: SampleFlags,
    composition_time_offset: i16,
}

/// Reads an optional `sample_flags` word, passing over one that sets a reserved bit
fn first_sample_flags(
    unstructured: &mut Unstructured<'_>,
) -> arbitrary::Result<Option<SampleFlags>> {
    unstructured
        .arbitrary::<Option<u32>>()?
        .map(|bits| SampleFlags::from_bits(bits).ok_or(arbitrary::Error::IncorrectFormat))
        .transpose()
}

/// One movie fragment as it lies in the presentation
pub struct Placed {
    pub movie_fragment: MovieFragmentBox,
    /// Where the `moof` begins in the presentation
    pub moof_start: u64,
    /// Where the data its samples claim begins, and the media data that meets it
    pub data: Option<(u64, Range<usize>)>,
}

/// One sample as the fragments declared it
pub struct Declared {
    track_id: u32,
    /// Bytes of the media data the sample is carried as
    data: Range<usize>,
}

/// A presentation laid out from one input
pub struct LaidOut {
    pub movie: MovieBox,
    pub fragments: Vec<Placed>,
    /// The samples the fragments declared, in the order they declared them
    declared: Vec<Declared>,
    /// Rows the fragments carry, however they lie
    pub rows: usize,
    /// Whether every fragment lies as the movie has it
    pub lies_as_declared: bool,
    /// Whether every fragment lies as the movie has it, every claim met
    pub met_as_declared: bool,
}

/// Lays `input` out as the presentation the fragments and their media data make
///
/// Reports `None` where the boxes of the input do not build: a fragment stating
/// what a box cannot carry is an input this run passes over rather than a failure
/// of the reader.
pub fn lay_out(input: &Input<'_>) -> Option<LaidOut> {
    let trex = input
        .track_defaults
        .iter()
        .enumerate()
        .map(|(position, defaults)| {
            TrackExtendsBox::new(
                track_id_of(position),
                SAMPLE_DESCRIPTION_INDEX,
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
    let mut all_lie_as_declared = true;
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
            let flags = if matches!(track_fragment.anchor, Anchor::MovieFragment) {
                TrackFragmentHeaderFlags::DEFAULT_BASE_IS_MOOF
            } else {
                TrackFragmentHeaderFlags::ZERO
            };
            let tfhd = TrackFragmentHeaderBox::new(
                flags,
                track_id,
                base_data_offset,
                track_fragment
                    .states_sample_description_index
                    .then_some(SAMPLE_DESCRIPTION_INDEX),
                states_defaults.then(|| first_row.map_or(0, |row| u32::from(row.duration))),
                states_defaults.then(|| first_row.map_or(0, |row| u32::from(row.size))),
                states_defaults.then(|| first_row.map_or(SampleFlags::ZERO, |row| row.flags)),
            );
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
                        CompositionTimeOffset::new(i64::from(row.composition_time_offset))
                            .filter(|_stated| track_run.states_composition_time_offset),
                    );

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

            // Why not `data_cursor` for every track fragment: one carrying no
            // run defines no data, so the next one anchored at the end of the
            // data before it (§8.8.7.1) starts where this one was anchored, as
            // the resolver has it.
            data_before = Some(if runs.is_empty() { anchor } else { data_cursor });
            let traf = if track_fragment.duration_is_empty {
                TrackFragmentBox::with_empty_duration(tfhd)
            } else {
                TrackFragmentBox::new(tfhd, runs)
            };
            track_fragments.push(match tfdt {
                Some(tfdt) => traf.with_tfdt(tfdt),
                None => traf,
            });
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

            Some((claim_start, laid_down.saturating_sub(arrived)..laid_down))
        } else {
            all_lie_as_declared = false;
            met_as_declared = false;
            None
        };

        fragments.push(Placed {
            movie_fragment: MovieFragmentBox::new(
                MovieFragmentHeaderBox::new(fragment.sequence_number),
                track_fragments,
            ),
            moof_start,
            data,
        });
        cursor = data_cursor;
    }

    Some(LaidOut {
        movie,
        fragments,
        declared,
        rows,
        lies_as_declared: all_lie_as_declared,
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

/// Returns whether every fragment leaves the decode time of its track to the durations before it
pub fn follows_by_durations(input: &Input<'_>) -> bool {
    input
        .fragments
        .iter()
        .take(MAX_FRAGMENTS)
        .flat_map(|fragment| fragment.track_fragments.iter().take(MAX_TRACK_FRAGMENTS))
        .all(|track_fragment| {
            track_fragment.base_media_decode_time.is_none() && !track_fragment.duration_is_empty
        })
}

impl LaidOut {
    /// The samples the fragments declared, by the bytes of the media data laid down for each
    pub fn declared_as(&self, media_data: &[u8]) -> Vec<(u32, Vec<u8>)> {
        self.declared
            .iter()
            .map(|declared| {
                let carried = media_data.get(declared.data.clone()).unwrap_or_default();

                (declared.track_id, carried.to_vec())
            })
            .collect()
    }
}
