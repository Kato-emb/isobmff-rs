//! Reading of [`Demuxer`] over a file cut into pieces
//!
//! The input is the bytes of a file as they lie, so a file of boxes is a seed
//! as it is. One run reads the file cut a few ways, and checks five
//! properties of it:
//!
//! 1. no read panics
//! 2. every cut reads to the same events, and ends in the same failure or
//!    none, as the file given whole. A read that fails on what the demuxer
//!    holds is compared up to the events the shorter read reports: what is
//!    held depends on the samples taken by then, which the cut decides
//! 3. a read that ends without a failure returns the sample of every location
//!    the events carry, in their order, holding the bytes of the file the
//!    location names
//! 4. the offset the demuxer wants goes back only after a call that reports a
//!    `moov` or a `moof`, whose samples may lie before it
//! 5. the read ends within as many calls as the file has bytes, the samples
//!    the events locate have bytes of media data, and two for each event:
//!    every call reads a byte of a box, keeps a byte of a sample, reports a
//!    box or fails, and a gathered box with an empty payload is wanted twice
//!    at one offset. The offset may stay put for longer, as it does at a
//!    sample that fits what the demuxer holds only once the samples held
//!    before it are taken

#![no_main]

use isobmff::{
    BoxType, ByteRange, DemuxError, DemuxEvent, DemuxLimits, Demuxer, Sample, SampleLocation,
};
use libfuzzer_sys::fuzz_target;

/// Lengths the file is cut into, each cut taking its lengths in turn, short ones followed by a long one
const CUTS: [&[usize]; 2] = [&[1, 4096], &[3, 7, 64, 1021]];

/// Limits that keep a run short
const LIMITS: DemuxLimits = DemuxLimits::new()
    .with_payload_bytes(1 << 20)
    .with_sample_count(1 << 16)
    .with_subsegment_count(1 << 16)
    .with_sample_bytes(1 << 20)
    .with_held_bytes(1 << 20);

/// Event as a caller records it
#[derive(Clone, PartialEq, Debug)]
enum Recorded {
    FileType(ByteRange),
    SegmentType(ByteRange),
    Movie(ByteRange, Vec<SampleLocation>),
    MovieFragment(ByteRange, Vec<SampleLocation>),
    SegmentIndex(ByteRange),
    MovieFragmentRandomAccess(ByteRange),
    MediaData(BoxType, ByteRange),
    Other(BoxType, ByteRange),
}

fuzz_target!(|bytes: &[u8]| {
    let whole = read(bytes, &[usize::MAX]);
    if whole.ending.is_ok() {
        let expected: Vec<_> = whole
            .events
            .iter()
            .flat_map(|event| match event {
                Recorded::Movie(_, locations) | Recorded::MovieFragment(_, locations) => {
                    locations.as_slice()
                }
                _ => &[],
            })
            .map(|location| {
                let byte_range = location.byte_range();
                let data = usize::try_from(byte_range.start)
                    .ok()
                    .zip(usize::try_from(byte_range.end).ok())
                    .and_then(|(start, end)| bytes.get(start..end))
                    .or(byte_range.is_empty().then_some(&[]))
                    .expect("a read without a failure locates a sample past the file");
                Sample::new(*location.properties(), data.to_vec())
            })
            .collect();
        assert!(
            whole.samples == expected,
            "the samples are not the bytes their locations name"
        );
    }
    for piece_lens in CUTS {
        let read = read(bytes, piece_lens);
        if fails_on_what_is_held(&read) || fails_on_what_is_held(&whole) {
            let shorter = read.events.len().min(whole.events.len());
            assert_eq!(
                read.events.get(..shorter),
                whole.events.get(..shorter),
                "pieces of {piece_lens:?} read otherwise than the whole file"
            );
            continue;
        }
        assert_eq!(
            (&read.events, &read.ending),
            (&whole.events, &whole.ending),
            "pieces of {piece_lens:?} read otherwise than the whole file"
        );
        if whole.ending.is_ok() {
            assert!(
                read.samples == whole.samples,
                "pieces of {piece_lens:?} return other samples than the whole file"
            );
        }
    }
});

/// Returns whether `read` fails on the locations or the bytes the demuxer holds
fn fails_on_what_is_held(read: &Read) -> bool {
    matches!(
        read.ending,
        Err(DemuxError::HeldLocationLimitExceeded { .. }
            | DemuxError::HeldBytesLimitExceeded { .. })
    )
}

/// What a read of a file saw
struct Read {
    events: Vec<Recorded>,
    samples: Vec<Sample>,
    ending: Result<(), DemuxError>,
}

/// Reads `bytes` from wherever the demuxer wants them, in pieces of `piece_lens` in turn
fn read(bytes: &[u8], piece_lens: &[usize]) -> Read {
    let mut recorded = Vec::new();
    let mut samples = Vec::new();
    let ending = (|| {
        let mut piece_lens = piece_lens.iter().cycle();
        let mut demuxer = Demuxer::with_limits(LIMITS);
        let mut last_wanted: Option<u64> = None;
        let mut located = false;
        let (mut calls, mut bound) = (0_u64, u64::try_from(bytes.len()).unwrap() + 2);
        while let Some(wanted) = demuxer.wanted_input() {
            let offset = wanted.offset();
            if let Some(last) = last_wanted
                && offset < last
                && !located
            {
                panic!("the demuxer wants {offset} after {last}");
            }
            last_wanted = Some(offset);
            calls += 1;
            assert!(
                calls <= bound,
                "the read takes {calls} calls, past the {bound} the file accounts for"
            );
            let rest = bytes
                .get(usize::try_from(offset).unwrap()..)
                .unwrap_or_default();
            let input = rest.get(..*piece_lens.next().unwrap()).unwrap_or(rest);
            let event = match input {
                [] => demuxer.finish()?,
                input => demuxer.handle_input(offset, input)?,
            };
            samples.extend(std::iter::from_fn(|| demuxer.poll_sample()));
            located = matches!(
                event,
                Some(DemuxEvent::Movie { .. } | DemuxEvent::MovieFragment { .. })
            );
            if let Some(event) = event {
                let event = match event {
                    DemuxEvent::FileType { byte_range, .. } => Recorded::FileType(byte_range),
                    DemuxEvent::SegmentType { byte_range, .. } => Recorded::SegmentType(byte_range),
                    DemuxEvent::Movie {
                        byte_range,
                        samples,
                        ..
                    } => Recorded::Movie(byte_range, samples.collect()),
                    DemuxEvent::MovieFragment {
                        byte_range,
                        samples,
                        ..
                    } => Recorded::MovieFragment(byte_range, samples.collect()),
                    DemuxEvent::SegmentIndex { byte_range, .. } => {
                        Recorded::SegmentIndex(byte_range)
                    }
                    DemuxEvent::MovieFragmentRandomAccess { byte_range, .. } => {
                        Recorded::MovieFragmentRandomAccess(byte_range)
                    }
                    DemuxEvent::MediaData {
                        header, byte_range, ..
                    } => Recorded::MediaData(header.box_type(), byte_range),
                    DemuxEvent::Other {
                        header, byte_range, ..
                    } => Recorded::Other(header.box_type(), byte_range),
                    event => panic!("unexpected event {event:?}"),
                };
                if let Recorded::Movie(_, locations) | Recorded::MovieFragment(_, locations) =
                    &event
                {
                    bound = locations
                        .iter()
                        .map(|location| {
                            let byte_range = location.byte_range();
                            byte_range.end - byte_range.start
                        })
                        .fold(bound, u64::saturating_add);
                }
                bound = bound.saturating_add(2);
                recorded.push(event);
            }
        }
        Ok(())
    })();
    Read {
        events: recorded,
        samples,
        ending,
    }
}
