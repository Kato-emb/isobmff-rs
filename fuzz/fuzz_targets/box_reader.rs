//! Reading of [`BoxReader`] over a file cut into pieces
//!
//! The input is the bytes of a file as they lie, so a file of boxes is a seed
//! as it is. The boxes are answered in turn: gathered, streamed, then skipped.
//! A gather the limit refuses is answered again with a skip. The events of the
//! boxes gathered or streamed go on to a [`BoxWriter`]. One run reads the file
//! cut a few ways, and checks three properties of it:
//!
//! 1. no read panics, and the writer takes every event it is passed
//! 2. every cut reads to the same events, payload pieces joined, and ends in
//!    the same failure or none, as the file given whole
//! 3. a read that ends without failure reads the boxes [`Boxes`] frames the
//!    file into, and at most one more: a skipped box that the file ends
//!    inside. Each piece of bytes the writer returns is the bytes the file
//!    holds where the box it writes lies

#![no_main]

use std::ops::Range;

use isobmff::framing::{BoxEvent, BoxReader, BoxWriter};
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
    Header(BoxHeader, Option<ByteRange>, Answer),
    Payload(Range<usize>),
    Gathered(Vec<u8>),
    End,
}

/// Writer the boxes gathered or streamed go on to, and the file offset the bytes it returns next lie at in the file
struct Copier {
    writer: BoxWriter,
    at: usize,
    /// Whether the box being read is gathered or streamed, and goes on to the writer
    copying: bool,
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

    let (recorded, ending) = &whole;
    if ending.is_ok() {
        let headers = recorded
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

/// Reads `bytes` from wherever the reader wants them, in pieces of `piece_lens` in turn, and returns the events and how the read ended
fn read(bytes: &[u8], piece_lens: &[usize]) -> (Vec<Recorded>, Result<(), DemuxError>) {
    let mut recorded = Vec::new();
    let mut copy = Copier {
        writer: BoxWriter::new(),
        at: 0,
        copying: false,
    };
    let ending = drive(bytes, piece_lens, &mut copy, &mut recorded);
    if ending.is_ok() {
        copy.writer.finish().unwrap();
    }
    (recorded, ending)
}

fn drive(
    bytes: &[u8],
    piece_lens: &[usize],
    copy: &mut Copier,
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
            record(event, bytes, &mut reader, &mut answers, copy, recorded)?;
        }
    }
    for event in reader.finish()? {
        record(event, bytes, &mut reader, &mut answers, copy, recorded)?;
    }
    Ok(())
}

fn record(
    event: BoxEvent<'_>,
    bytes: &[u8],
    reader: &mut BoxReader,
    answers: &mut impl Iterator<Item = Answer>,
    copy: &mut Copier,
    recorded: &mut Vec<Recorded>,
) -> Result<(), DemuxError> {
    match event {
        BoxEvent::Header(header) => {
            let byte_range = reader.open_box();
            let mut answer = answers.next().unwrap();
            match answer {
                Answer::Gather => {
                    if reader.gather(GATHER_LIMIT).is_err() {
                        reader.skip()?;
                        answer = Answer::Skip;
                    }
                }
                Answer::Stream => reader.stream()?,
                Answer::Skip => reader.skip()?,
            }
            copy.at = usize::try_from(byte_range.unwrap().offset()).unwrap();
            copy.copying = answer != Answer::Skip;
            recorded.push(Recorded::Header(header, byte_range, answer));
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
        BoxEvent::End => {
            if let Some(raw_box) = reader.gathered_box() {
                recorded.push(Recorded::Gathered(raw_box.payload().to_vec()));
                write(copy, bytes, BoxEvent::Payload(raw_box.payload()));
            }
            recorded.push(Recorded::End);
        }
        event => panic!("unknown event {event:?}"),
    }
    if copy.copying {
        write(copy, bytes, event);
    }
    Ok(())
}

/// Passes `event` on to the writer, and checks that the bytes it returns are those the file holds where they lie
fn write(copy: &mut Copier, bytes: &[u8], event: BoxEvent<'_>) {
    let returned = copy.writer.handle_event(event).unwrap();
    let end = copy.at + returned.len();
    assert_eq!(
        Some(returned),
        bytes.get(copy.at..end),
        "the writer returned other bytes than the file holds at {}",
        copy.at
    );
    copy.at = end;
}
