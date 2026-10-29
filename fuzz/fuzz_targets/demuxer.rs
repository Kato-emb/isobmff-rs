//! Reading of [`Demuxer`] over a file cut into pieces
//!
//! The input is the bytes of a file as they lie, so a file of boxes is a seed
//! as it is. Every media data box is skipped, as a caller that can seek skips
//! it. One run reads the file cut a few ways, and checks three properties of
//! it:
//!
//! 1. no read panics
//! 2. every cut reads to the same events, and ends in the same failure or
//!    none, as the file given whole
//! 3. the offset the demuxer wants never goes back, and stays put for at most
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

/// Event as a caller records it
#[derive(Clone, PartialEq, Debug)]
enum Recorded {
    FileType,
    SegmentType,
    Movie(Vec<SampleLocation>),
    MovieFragment(Vec<SampleLocation>),
    SegmentIndex,
    MovieFragmentRandomAccess,
    MediaData(BoxType, ByteRange),
    Other(BoxType, ByteRange),
}

fuzz_target!(|bytes: &[u8]| {
    let whole = read(bytes, &[usize::MAX]);
    for piece_lens in CUTS {
        assert_eq!(
            read(bytes, piece_lens),
            whole,
            "pieces of {piece_lens:?} read otherwise than the whole file"
        );
    }
});

/// Reads `bytes` from wherever the demuxer wants them, in pieces of `piece_lens` in turn, and returns the events and how the read ended
fn read(bytes: &[u8], piece_lens: &[usize]) -> (Vec<Recorded>, Result<(), DemuxError>) {
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
                recorded.push(match event {
                    DemuxEvent::FileType => Recorded::FileType,
                    DemuxEvent::SegmentType => Recorded::SegmentType,
                    DemuxEvent::Movie { samples, .. } => Recorded::Movie(samples.collect()),
                    DemuxEvent::MovieFragment { samples, .. } => {
                        Recorded::MovieFragment(samples.collect())
                    }
                    DemuxEvent::SegmentIndex => Recorded::SegmentIndex,
                    DemuxEvent::MovieFragmentRandomAccess => Recorded::MovieFragmentRandomAccess,
                    DemuxEvent::MediaData {
                        header, byte_range, ..
                    } => {
                        demuxer.skip()?;
                        Recorded::MediaData(header.box_type(), byte_range)
                    }
                    DemuxEvent::Other {
                        header, byte_range, ..
                    } => Recorded::Other(header.box_type(), byte_range),
                    event => panic!("unexpected event {event:?}"),
                });
            }
        }
        Ok(())
    })();
    (recorded, ending)
}
