//! Reading of [`Demuxer`] over a file cut into pieces
//!
//! The input is the bytes of a file as they lie, so a file of boxes is a seed
//! as it is. One run reads the file cut a few ways, once skipping every media
//! data box, as a caller that can seek does, and once streaming it, and checks
//! four properties of it:
//!
//! 1. no read panics
//! 2. every cut reads to the same events, payload pieces joined, and ends in
//!    the same failure or none, as the file given whole
//! 3. each streamed payload is the bytes the file holds where it says it lies
//! 4. the offset the demuxer wants never goes back, and stays put for at most
//!    one call: the one that closes a gathered box with an empty payload

#![no_main]

use isobmff::{BoxType, ByteRange, DemuxError, DemuxEvent, DemuxLimits, Demuxer, SampleLocation};
use libfuzzer_sys::fuzz_target;

/// Lengths the file is cut into, each cut taking its lengths in turn, short ones followed by a long one
const CUTS: [&[usize]; 2] = [&[1, 4096], &[3, 7, 64, 1021]];

/// Limits that keep a run short
const LIMITS: DemuxLimits = DemuxLimits::new()
    .with_payload_bytes(1 << 20)
    .with_sample_count(1 << 16)
    .with_subsegment_count(1 << 16);

/// Event as a caller records it, a streamed payload by its file offset and bytes
#[derive(Clone, PartialEq, Debug)]
enum Recorded {
    FileType(ByteRange),
    SegmentType(ByteRange),
    Movie(ByteRange, Vec<SampleLocation>),
    MovieFragment(ByteRange, Vec<SampleLocation>),
    SegmentIndex(ByteRange),
    MovieFragmentRandomAccess(ByteRange),
    MediaData(BoxType, ByteRange),
    Payload(u64, Vec<u8>),
    Other(BoxType, ByteRange),
}

fuzz_target!(|bytes: &[u8]| {
    for stream in [false, true] {
        let whole = read(bytes, &[usize::MAX], stream);
        for piece_lens in CUTS {
            assert_eq!(
                read(bytes, piece_lens, stream),
                whole,
                "pieces of {piece_lens:?} read otherwise than the whole file"
            );
        }
        for recorded in &whole.0 {
            if let Recorded::Payload(offset, payload) = recorded {
                let start = usize::try_from(*offset).unwrap();
                assert_eq!(
                    bytes.get(start..start + payload.len()),
                    Some(payload.as_slice()),
                    "the payload at {offset} is not the bytes of the file there"
                );
            }
        }
    }
});

/// Reads `bytes` from wherever the demuxer wants them, in pieces of `piece_lens` in turn, streaming each media data box if `stream`, else skipping it, and returns the events and how the read ended
fn read(
    bytes: &[u8],
    piece_lens: &[usize],
    stream: bool,
) -> (Vec<Recorded>, Result<(), DemuxError>) {
    let mut recorded = Vec::new();
    let ending = (|| {
        let mut piece_lens = piece_lens.iter().cycle();
        let mut demuxer = Demuxer::with_limits(LIMITS);
        let mut last_wanted: Option<(u64, usize)> = None;
        while let Some(wanted) = demuxer.wanted_input() {
            let offset = wanted.offset();
            last_wanted = match last_wanted {
                Some((last, _)) if offset < last => {
                    panic!("the demuxer wants {offset} after {last}")
                }
                Some((last, times)) if offset == last => {
                    assert!(times < 2, "the demuxer wants {offset} a third time running");
                    Some((offset, times + 1))
                }
                _ => Some((offset, 1)),
            };
            let rest = bytes
                .get(usize::try_from(offset).unwrap()..)
                .unwrap_or_default();
            let input = rest.get(..*piece_lens.next().unwrap()).unwrap_or(rest);
            let events = match input {
                [] => demuxer.finish()?,
                input => demuxer.handle_input(offset, input)?,
            };
            if let Some(event) = events {
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
                    } => {
                        if stream {
                            demuxer.stream()?;
                        } else {
                            demuxer.skip()?;
                        }
                        Recorded::MediaData(header.box_type(), byte_range)
                    }
                    DemuxEvent::MediaDataPayload { offset, bytes, .. } => {
                        if let Some(Recorded::Payload(start, joined)) = recorded.last_mut()
                            && *start + joined.len() as u64 == offset
                        {
                            joined.extend_from_slice(bytes);
                            continue;
                        }
                        Recorded::Payload(offset, bytes.to_vec())
                    }
                    DemuxEvent::Other {
                        header, byte_range, ..
                    } => Recorded::Other(header.box_type(), byte_range),
                    event => panic!("unexpected event {event:?}"),
                };
                recorded.push(event);
            }
        }
        Ok(())
    })();
    (recorded, ending)
}
