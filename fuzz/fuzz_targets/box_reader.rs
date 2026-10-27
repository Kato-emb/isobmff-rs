//! Reading of [`BoxReader`] over a file cut into pieces
//!
//! The input is the bytes of a file as they lie, so a file of boxes is a seed
//! as it is. The boxes are answered in turn: gathered, streamed, then skipped.
//! One run reads the file cut a few ways, and checks two properties of it:
//!
//! 1. no read panics
//! 2. every cut reads to the same events, payload pieces joined, and ends in
//!    the same failure or none, as the file given whole
//!
//! A read that ends without failure reads the boxes [`Boxes`] frames the file
//! into, and at most one more: a skipped box that the file ends inside.

#![no_main]

use std::ops::Range;

use isobmff::framing::{BoxEvent, BoxReader};
use isobmff::{BoxHeader, Boxes, ByteRange, DemuxError};
use libfuzzer_sys::fuzz_target;

/// Lengths the file is cut into, each cut taking its lengths in turn, short ones followed by a long one
const CUTS: [&[usize]; 2] = [&[1, 4096], &[3, 7, 64, 1021]];

/// Limit a gathered payload is held to
const GATHER_LIMIT: u64 = 1024;

/// What a caller answers a header with
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Answer {
    Gather,
    Stream,
    Skip,
}

/// Event as a caller records it, a streamed payload as the range of the input it lies in
#[derive(Clone, PartialEq, Eq, Debug)]
enum Recorded {
    Header(BoxHeader, ByteRange, Answer),
    Payload(Range<usize>),
    Gathered(Vec<u8>),
    End,
}

fuzz_target!(|bytes: &[u8]| {
    let mut whole = Vec::new();
    let ending = read(bytes, &[usize::MAX], &mut whole);
    for piece_lens in CUTS {
        let mut pieces = Vec::new();
        let pieces_ending = read(bytes, piece_lens, &mut pieces);
        assert_eq!(
            (pieces, pieces_ending),
            (whole.clone(), ending),
            "pieces of {piece_lens:?} read otherwise than the whole file"
        );
    }

    if ending.is_ok() {
        let headers = whole
            .iter()
            .filter_map(|recorded| match recorded {
                Recorded::Header(header, _, answer) => Some((*header, *answer)),
                _ => None,
            })
            .collect::<Vec<_>>();
        let framed = Boxes::new(bytes)
            .map_while(Result::ok)
            .map(|framed| framed.header())
            .collect::<Vec<_>>();
        let (read_framed, past_input) = headers.split_at(framed.len().min(headers.len()));
        assert!(
            read_framed
                .iter()
                .map(|(header, _)| *header)
                .eq(framed.iter().copied()),
            "boxes read otherwise than they frame"
        );
        assert!(
            matches!(past_input, [] | [(_, Answer::Skip)]),
            "{past_input:?} read past what frames without being skipped"
        );
    }
});

/// Reads `bytes` from wherever the reader wants them, in pieces of `piece_lens` in turn, into `recorded`, and returns how the read ended
fn read(
    bytes: &[u8],
    piece_lens: &[usize],
    recorded: &mut Vec<Recorded>,
) -> Result<(), DemuxError> {
    let mut piece_lens = piece_lens.iter().cycle();
    let mut answers = [Answer::Gather, Answer::Stream, Answer::Skip]
        .into_iter()
        .cycle();
    let mut reader = BoxReader::new();
    while let Some(wanted) = reader.wanted_input() {
        let rest = bytes
            .get(usize::try_from(wanted.offset()).unwrap()..)
            .unwrap_or_default();
        let input = rest.get(..*piece_lens.next().unwrap()).unwrap_or(rest);
        if input.is_empty() {
            break;
        }
        for event in reader.handle_input(wanted.offset(), input)? {
            record(event?, bytes, &mut answers, recorded)?;
        }
    }
    for event in reader.finish()? {
        record(event?, bytes, &mut answers, recorded)?;
    }
    Ok(())
}

fn record(
    event: BoxEvent<'_>,
    bytes: &[u8],
    answers: &mut impl Iterator<Item = Answer>,
    recorded: &mut Vec<Recorded>,
) -> Result<(), DemuxError> {
    match event {
        BoxEvent::Header(open) => {
            let answer = answers.next().unwrap();
            recorded.push(Recorded::Header(open.header(), open.byte_range(), answer));
            match answer {
                Answer::Gather => open.gather(GATHER_LIMIT)?,
                Answer::Stream => open.stream(),
                Answer::Skip => open.skip(),
            }
        }
        BoxEvent::Payload(piece) => {
            // Why not copy the piece: a payload is borrowed from the input, so
            // where it lies in the input says both what it holds and that it
            // was not copied, without the cost of copying it.
            let start = (piece.as_ptr() as usize) - (bytes.as_ptr() as usize);
            let range = start..start + piece.len();
            match recorded.last_mut() {
                Some(Recorded::Payload(joined)) if joined.end == range.start => {
                    joined.end = range.end;
                }
                _ => recorded.push(Recorded::Payload(range)),
            }
        }
        BoxEvent::Gathered(raw_box) => {
            recorded.push(Recorded::Gathered(raw_box.payload().to_vec()))
        }
        BoxEvent::End => recorded.push(Recorded::End),
        event => panic!("unknown event {event:?}"),
    }
    Ok(())
}
