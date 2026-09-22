//! Gathering properties of [`SampleReader`] on its own
//!
//! One run declares extents over a file and hands the reader pieces of that
//! file, each with the offset it lies at, in whatever order the input has them,
//! and checks four properties of the same input, once as given, once with every
//! piece handed over twice, and once with every piece cut in two, each run of
//! extents held one at a time and then together — which holds the run in the
//! order of their bytes, so the model follows that order:
//!
//! 1. no call panics, a failure of the reader is reported again by every call
//!    after it, and once the samples are declared over nothing more is taken
//! 2. the samples handed over are the extents whose bytes arrived, each
//!    carrying the bytes of the file its extent named, and in the order the
//!    contract states: an extent is whole once every piece it takes from has
//!    arrived — a piece reaching the byte it lacks next fills it up to the end
//!    of the piece or of the extent — the whole extents come out in the order
//!    they were held, at the step that made them whole, and an extent naming
//!    no bytes once nothing short is held before it
//! 3. the samples are declared over without failure exactly when every extent
//!    held was met, the failure naming the short extent at the front of those held,
//!    and no want is named then
//! 4. an extent naming more bytes than the limit is refused, and none is held
//!    after it
//!
//! The order and the failure are read off a model of the contract that walks
//! the extents one at a time, so what the reader does with the extents held
//! together is checked against what each of them was told.

#![no_main]

use core::ops::Range;

use isobmff::{Sample, SampleError, SampleErrorKind, SampleExtent, SampleReader};
use libfuzzer_sys::arbitrary::{self, Arbitrary};
use libfuzzer_sys::fuzz_target;

/// Steps one run takes at most
const MAX_STEPS: usize = 64;

/// Input of one run: the limit, the steps, and the file the extents lie over
#[derive(Arbitrary, Debug)]
struct Input<'bytes> {
    /// Bytes one sample may declare, or the reader's own limit
    sample_size_limit: Option<u16>,
    steps: Vec<Step>,
    // Why not put `file` first, and why a slice: only the last field is handed
    // what is left, and only `&[u8]` takes it verbatim — a `Vec<u8>` reads a
    // byte of its own before each element, so a seed stops at its first even
    // byte.
    file: &'bytes [u8],
}

/// What a caller hands the reader next
#[derive(Arbitrary, Debug, Clone, Copy)]
enum Step {
    /// An extent to hold
    Extent {
        track_id: u8,
        decode_time: u32,
        sample_duration: u16,
        sample_composition_time_offset: i8,
        sample_flags: u8,
        /// Where in the file the extent starts
        start: u16,
        /// Bytes the extent names
        len: u16,
    },
    /// A piece of the file, `len` bytes of it from `offset` on
    Data {
        offset: u16,
        len: u8,
        /// Where the piece is cut in two, in the pass that cuts them
        cut_at: u8,
    },
}

/// One step as the reader and the model take it
enum Handed<'file> {
    /// A run of extents, held one at a time or together
    Extents(Vec<SampleExtent>),
    Data(u64, &'file [u8]),
}

/// Everything one pass over the steps reported
#[derive(PartialEq, Debug)]
struct Reading {
    samples: Vec<Sample>,
    failure: Option<SampleError>,
}

/// An extent held, as the model follows it through the steps
struct Followed {
    extent: SampleExtent,
    gathered: u64,
    /// The step that made the extent whole, if one did
    whole_at: Option<usize>,
}

fuzz_target!(|input: Input<'_>| {
    let sample_size_limit = input
        .sample_size_limit
        .map_or(SampleReader::DEFAULT_SAMPLE_SIZE_LIMIT, u64::from);
    let steps: Vec<Step> = input.steps.iter().copied().take(MAX_STEPS).collect();

    for pass in [Pass::AsGiven, Pass::Twice, Pass::Cut] {
        let handed = handed_over(&steps, input.file, pass);

        for together in [false, true] {
            assert_eq!(
                read(sample_size_limit, &handed, together),
                modelled(sample_size_limit, &handed, input.file, together),
                "the reader did not do with the extents held {} what the contract states for each",
                if together { "together" } else { "one at a time" }
            );
        }
    }
});

/// How the pieces of the file reach the reader
#[derive(Clone, Copy)]
enum Pass {
    /// Each piece once, as the input has them
    AsGiven,
    /// Each piece twice over
    Twice,
    /// Each piece in two, cut where the input says
    Cut,
}

/// The steps as the reader takes them, the pieces of the file handed over as `pass` has them
fn handed_over<'file>(steps: &[Step], file: &'file [u8], pass: Pass) -> Vec<Handed<'file>> {
    let mut handed = Vec::new();
    let mut run = Vec::new();

    for step in steps {
        match *step {
            Step::Extent {
                track_id,
                decode_time,
                sample_duration,
                sample_composition_time_offset,
                sample_flags,
                start,
                len,
            } => {
                let start = u64::from(start);

                run.push(SampleExtent::new(
                    u32::from(track_id),
                    u64::from(decode_time),
                    u32::from(sample_duration),
                    i64::from(sample_composition_time_offset),
                    u32::from(sample_flags),
                    1,
                    1,
                    start..start.saturating_add(u64::from(len)),
                ));
            }
            Step::Data {
                offset,
                len,
                cut_at,
            } => {
                let from = usize::from(offset).min(file.len());
                let to = from.saturating_add(usize::from(len)).min(file.len());
                let piece = &file[from..to];
                let offset = u64::from(offset);

                if !run.is_empty() {
                    handed.push(Handed::Extents(std::mem::take(&mut run)));
                }
                match pass {
                    Pass::AsGiven => handed.push(Handed::Data(offset, piece)),
                    Pass::Twice => {
                        handed.push(Handed::Data(offset, piece));
                        handed.push(Handed::Data(offset, piece));
                    }
                    Pass::Cut => {
                        let cut = usize::from(cut_at).min(piece.len());
                        let (first, second) = piece.split_at(cut);

                        handed.push(Handed::Data(offset, first));
                        handed.push(Handed::Data(
                            offset.saturating_add(u64::try_from(cut).unwrap_or(0)),
                            second,
                        ));
                    }
                }
            }
        }
    }
    if !run.is_empty() {
        handed.push(Handed::Extents(run));
    }

    handed
}

/// Hands the steps to a reader and gathers what it reports
///
/// A run of extents is handed over in one call where `together` is set, and
/// one at a time otherwise.
fn read(sample_size_limit: u64, handed: &[Handed<'_>], together: bool) -> Reading {
    let mut reader = SampleReader::with_sample_size_limit(sample_size_limit);
    let mut samples = Vec::new();
    let mut failure = None;

    for step in handed {
        let outcome = match step {
            Handed::Extents(run) if together => {
                reader.handle_sample_extents(run.iter().cloned().map(Ok))
            }
            Handed::Extents(run) => run
                .iter()
                .try_for_each(|extent| reader.handle_sample_extent(extent.clone())),
            Handed::Data(offset, data) => reader.handle_data(*offset, data),
        };

        drain(&mut reader, &mut samples);

        if let Err(reported) = outcome {
            failure = Some(reported);
            break;
        }
    }

    if failure.is_none() {
        failure = reader.finish().err();
        drain(&mut reader, &mut samples);
    }

    match failure {
        Some(reported) => {
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
        }
        None => {
            assert_eq!(
                reader.handle_data(0, &[]).map_err(SampleError::kind),
                Err(SampleErrorKind::AlreadyFinished),
                "the reader took media data after the samples were declared over"
            );
            assert_eq!(
                reader.wanted_extent(),
                None,
                "the reader wanted bytes after the samples were declared over without failure"
            );
        }
    }

    Reading { samples, failure }
}

/// Takes every sample the reader has completed
fn drain(reader: &mut SampleReader, samples: &mut Vec<Sample>) {
    while let Some(sample) = reader.poll_sample() {
        samples.push(sample);
    }
}

/// What the contract states the reader reports for the steps, each extent followed on its own
///
/// A run of extents held `together` is held in the order of their bytes, up
/// to the first past the limit, which is refused before anything after it.
fn modelled(sample_size_limit: u64, handed: &[Handed<'_>], file: &[u8], together: bool) -> Reading {
    let mut followed: Vec<Followed> = Vec::new();
    let mut failure = None;

    for (step, handed) in handed.iter().enumerate() {
        match handed {
            Handed::Extents(run) => {
                let admitted = run
                    .iter()
                    .position(|extent| declared_len(extent) > sample_size_limit)
                    .unwrap_or(run.len());
                let mut held: Vec<&SampleExtent> = run[..admitted].iter().collect();
                if together {
                    held.sort_by_key(|extent| extent.extent().start);
                }
                followed.extend(held.into_iter().map(|extent| Followed {
                    extent: extent.clone(),
                    gathered: 0,
                    whole_at: (declared_len(extent) == 0).then_some(step),
                }));
                if let Some(refused) = run.get(admitted) {
                    failure = Some(SampleError::sample_size_limit_exceeded(
                        refused.track_id(),
                        declared_len(refused),
                        sample_size_limit,
                    ));
                    break;
                }
            }
            Handed::Data(offset, data) => {
                let arriving = *offset..offset.saturating_add(data.len() as u64);

                for pending in followed.iter_mut().filter(|pending| pending.whole_at.is_none()) {
                    let named = pending.extent.extent();
                    let lacking_from = named.start.saturating_add(pending.gathered);
                    if arriving.contains(&lacking_from) {
                        pending.gathered = arriving.end.min(named.end).saturating_sub(named.start);
                        if pending.gathered >= declared_len(&pending.extent) {
                            pending.whole_at = Some(step);
                        }
                    }
                }
            }
        }
    }

    if failure.is_none() {
        failure = followed
            .iter()
            .find(|pending| pending.whole_at.is_none())
            .map(|short| {
                SampleError::unfinished_sample(
                    short.extent.track_id(),
                    declared_len(&short.extent),
                    short.gathered,
                )
            });
    }

    // Why not reporting every extent at the step that made it whole: one
    // naming no bytes is whole where it is held, and comes out only once
    // nothing short is held before it
    let mut reported: Vec<(usize, &Followed)> = Vec::new();
    let mut cleared_at = Some(0);
    for pending in &followed {
        cleared_at = cleared_at
            .zip(pending.whole_at)
            .map(|(cleared, whole)| cleared.max(whole));
        let reported_at = if declared_len(&pending.extent) == 0 {
            cleared_at
        } else {
            pending.whole_at
        };
        if let Some(reported_at) = reported_at {
            reported.push((reported_at, pending));
        }
    }
    reported.sort_by_key(|(reported_at, _pending)| *reported_at);

    Reading {
        samples: reported
            .into_iter()
            .map(|(_reported_at, pending)| sample_of(&pending.extent, file))
            .collect(),
        failure,
    }
}

/// Returns the bytes `extent` names
fn declared_len(extent: &SampleExtent) -> u64 {
    let named: Range<u64> = extent.extent();

    named.end.saturating_sub(named.start)
}

/// The sample `extent` names, carrying the bytes of `file` it lies over
fn sample_of(extent: &SampleExtent, file: &[u8]) -> Sample {
    let named = extent.extent();
    let data = usize::try_from(named.start)
        .ok()
        .zip(usize::try_from(named.end).ok())
        .and_then(|(start, end)| file.get(start..end))
        .unwrap_or_default();

    Sample::new(
        extent.track_id(),
        extent.decode_time(),
        extent.sample_duration(),
        extent.sample_composition_time_offset(),
        extent.sample_flags(),
        extent.sample_description_index(),
        data.to_vec(),
    )
}
