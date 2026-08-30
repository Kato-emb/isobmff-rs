//! Reading properties of [`BoxReader`]
//!
//! One run checks five properties of the same input:
//!
//! 1. no call panics, and no event carries a payload part that is empty
//! 2. the extents partition the input the reader got through: each one begins
//!    where the one before it ended, and a box lies over the header, the
//!    payload parts, and the empty end it was read as
//! 3. where the input is cut does not change the boxes it reads, nor where they lie
//! 4. the boxes read agree with the [`boxes`] iterator over the input — header,
//!    payload, and the offset each box begins at — as far as the reader got
//!    through it
//! 5. input the iterator rejects the reader rejects as well

#![no_main]

use isobmff_core::{BoxHeader, boxes};
use isobmff_sequence::{BoxEvent, BoxReader};
use libfuzzer_sys::arbitrary::{self, Arbitrary};
use libfuzzer_sys::fuzz_target;

#[path = "helpers/cut.rs"]
mod cut;

use cut::cut_into;

/// Input of one run: bytes to read, and the lengths to cut them into
///
/// A corpus file is four bytes of `cut_lengths` followed by the bytes
/// themselves, verbatim. The lengths are cycled, each one byte longer than it
/// reads, so a cut is 1 to 256 bytes and the feeding always advances.
#[derive(Arbitrary, Debug)]
struct Input<'bytes> {
    // Why not put `bytes` first, and why a slice: only the last field is handed
    // what is left, and only `&[u8]` takes it verbatim — a `Vec<u8>` reads a
    // byte of its own before each element, so a seed stops at its first even
    // byte.
    cut_lengths: [u8; 4],
    bytes: &'bytes [u8],
}

/// One box as the reader reported it, gathered back from its events
#[derive(PartialEq, Debug)]
struct Reported {
    header: BoxHeader,
    began_at: u64,
    payload: Vec<u8>,
    ended: bool,
}

/// Everything one pass of the reader over an input reported
#[derive(PartialEq, Debug)]
struct Run {
    reported: Vec<Reported>,
    covered: u64,
    // Why the failure as its own text: the error carries the failure of a box
    // that did not decode, which is not a value two runs can be compared by.
    failure: Option<String>,
}

fuzz_target!(|input: Input<'_>| {
    let Input { cut_lengths, bytes } = input;

    let whole = read([bytes]);
    let cut = read(cut_into(bytes, cut_lengths));

    assert_eq!(
        whole, cut,
        "where the input was cut changed the boxes it read, or where they lie"
    );

    if whole.failure.is_none() {
        assert_eq!(
            whole.covered,
            u64::try_from(bytes.len()).expect("an input beyond `u64` was read"),
            "the extents stop short of an input the reader read to the end of"
        );
    }

    agrees_with_the_boxes_iterator(bytes, &whole);
});

/// Hands `arriving` to a reader, a part at a time, and gathers what it reports
fn read<'input>(arriving: impl IntoIterator<Item = &'input [u8]>) -> Run {
    let mut reader = BoxReader::new();
    let mut reported: Vec<Reported> = Vec::new();
    let mut covered = 0;
    let mut failure = None;

    for input in arriving {
        // Why drain before reporting the failure: the events made before it are
        // still the reader's to hand over, and dropping them would leave a box
        // half gathered for the comparison against the iterator.
        let outcome = reader.handle_input(input);
        drain(&mut reader, &mut reported, &mut covered);

        if let Err(reported) = outcome {
            failure = Some(format!("{reported:?}"));
            break;
        }
    }

    if failure.is_none() {
        let outcome = reader.finish();
        drain(&mut reader, &mut reported, &mut covered);

        if let Err(reported) = outcome {
            failure = Some(format!("{reported:?}"));
        }
    }

    // Why not keep the box left unclosed: it spans bytes the reader never got
    // through, so comparing it against the iterator would hold a partial box up
    // against a whole one.
    reported.retain(|reported| reported.ended);

    Run {
        reported,
        covered,
        failure,
    }
}

/// Takes every event the reader has made, folding it into the boxes gathered
///
/// `covered` walks the input along with the extents, each event taking it from
/// where the one before it left off to the end of the bytes that event was read
/// from.
fn drain(reader: &mut BoxReader, reported: &mut Vec<Reported>, covered: &mut u64) {
    while let Some(polled) = reader.poll_event() {
        let extent = reader
            .event_extent()
            .expect("an event was taken, so it has an extent");
        let began_at = extent.start;
        let bytes_read_from = extent
            .end
            .checked_sub(extent.start)
            .expect("an extent ending before it begins");

        assert_eq!(
            began_at, *covered,
            "an event does not begin where the one before it ended"
        );
        *covered = extent.end;

        match polled {
            BoxEvent::Header(header) => {
                assert_eq!(
                    bytes_read_from,
                    u64::try_from(header.encoded_len()).expect("a header beyond `u64` was read"),
                    "the extent of a box starting is not the header it was read from"
                );
                reported.push(Reported {
                    header,
                    began_at,
                    payload: Vec::new(),
                    ended: false,
                });
            }
            BoxEvent::Payload(part) => {
                assert!(!part.is_empty(), "an empty payload event was reported");
                assert_eq!(
                    bytes_read_from,
                    u64::try_from(part.len()).expect("a payload part beyond `u64` was read"),
                    "the extent of a payload part is not the bytes it carries"
                );
                reported
                    .last_mut()
                    .expect("a payload before any box started")
                    .payload
                    .extend_from_slice(&part);
            }
            BoxEvent::End => {
                assert_eq!(
                    bytes_read_from, 0,
                    "the extent of a box ending covers bytes of the input"
                );
                reported.last_mut().expect("an end before any box started").ended = true;
            }
            unknown => panic!("the reader reported an event this run cannot check: {unknown:?}"),
        }
    }
}

fn agrees_with_the_boxes_iterator(bytes: &[u8], run: &Run) {
    let mut iterated = Vec::new();
    let mut iterator_failed = false;

    for framed in boxes(bytes) {
        match framed {
            Ok(framed) => iterated.push(framed),
            Err(_) => iterator_failed = true,
        }
    }

    if run.failure.is_none() {
        assert_eq!(
            run.reported.len(),
            iterated.len(),
            "the reader and the iterator disagree on how many boxes the input holds"
        );
    } else {
        assert!(
            run.reported.len() <= iterated.len(),
            "the reader reported boxes the iterator does not split out"
        );
    }

    if iterator_failed {
        assert!(
            run.failure.is_some(),
            "the reader read to the end of input the iterator rejects"
        );
    }

    let mut offset = 0;

    for (reported, framed) in run.reported.iter().zip(&iterated) {
        assert_eq!(
            reported.began_at, offset as u64,
            "a box was reported at an offset it does not begin at"
        );
        assert_eq!(
            reported.header,
            framed.header(),
            "the reader and the iterator disagree on the header of a box"
        );
        assert_eq!(
            reported.payload.as_slice(),
            framed.payload(),
            "the reader and the iterator disagree on the payload of a box"
        );

        offset += framed.header().encoded_len() + framed.payload().len();
    }
}
