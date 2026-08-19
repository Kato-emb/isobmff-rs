//! Reading properties of [`BoxReader`]
//!
//! One run checks four properties of the same input:
//!
//! 1. no call panics, and no event carries a payload part that is empty
//! 2. where the input is cut does not change the boxes it reads
//! 3. the boxes read agree with the [`boxes`] iterator over the input — box type,
//!    header, payload, and the offset each box begins at — as far as the reader
//!    got through it
//! 4. input the iterator rejects the reader rejects as well
//!
//! Where the reader alone rejects, property 3 holds over the boxes it did report
//! rather than the whole input: reading a box into a value decodes its payload,
//! which the iterator never looks at. That the boxes the iterator splits out
//! re-encode to the spans they came from is the `boxes` target's property, not
//! this one's.

#![no_main]

use isobmff_core::{BoxHeader, BoxType, boxes};
use isobmff_sequence::{BoxEvent, BoxReader};
use libfuzzer_sys::arbitrary::{self, Arbitrary};
use libfuzzer_sys::fuzz_target;

/// Input of one run: bytes to read, and the lengths to cut them into
///
/// A corpus file is four bytes of `cut_lengths` followed by the bytes
/// themselves, verbatim. The lengths are cycled, each one byte longer than it
/// reads, so a cut is 1 to 256 bytes and the feeding always advances.
#[derive(Arbitrary, Debug)]
struct Input {
    // Why not put `bytes` first: only a trailing `Vec<u8>` is taken verbatim by
    // `Arbitrary`, so any other order leaves the seed files unreadable as a
    // hexdump of the input.
    cut_lengths: [u8; 4],
    bytes: Vec<u8>,
}

/// One box as the reader reported it
#[derive(PartialEq, Debug)]
enum Reported {
    /// Box passed on as it lies, gathered back from its raw events
    PassedOn {
        header: BoxHeader,
        file_offset: u64,
        payload: Vec<u8>,
        ended: bool,
    },
    /// Box read into a value, which publishes no bytes of its own
    Value { box_type: BoxType, file_offset: u64 },
}

/// Everything one pass of the reader over an input reported
#[derive(PartialEq, Debug)]
struct Run {
    reported: Vec<Reported>,
    // Why the failure as its own text: the error carries the failure of a box
    // that did not decode, which is not a value two runs can be compared by.
    failure: Option<String>,
}

fuzz_target!(|input: Input| {
    let Input { cut_lengths, bytes } = input;

    let whole = read([bytes.as_slice()]);
    let cut = read(cut_into(&bytes, cut_lengths));

    assert_eq!(
        whole, cut,
        "where the input was cut changed the boxes it read"
    );

    agrees_with_the_boxes_iterator(&bytes, &whole);
});

/// Hands `arriving` to a reader, a part at a time, and gathers what it reports
fn read<'input>(arriving: impl IntoIterator<Item = &'input [u8]>) -> Run {
    let mut reader = BoxReader::new();
    let mut reported: Vec<Reported> = Vec::new();
    let mut failure = None;

    for input in arriving {
        // Why drain before reporting the failure: the events made before it are
        // still the reader's to hand over, and dropping them would leave a box
        // half gathered for the comparison against the iterator.
        let outcome = reader.handle_input(input);
        drain(&mut reader, &mut reported);

        if let Err(reported) = outcome {
            failure = Some(format!("{reported:?}"));
            break;
        }
    }

    if failure.is_none() {
        let outcome = reader.finish();
        drain(&mut reader, &mut reported);

        if let Err(reported) = outcome {
            failure = Some(format!("{reported:?}"));
        }
    }

    // Why not keep the box left unclosed: it spans bytes the reader never got
    // through, so comparing it against the iterator would hold a partial box up
    // against a whole one.
    reported.retain(|reported| match *reported {
        Reported::PassedOn { ended, .. } => ended,
        Reported::Value { .. } => true,
    });

    Run { reported, failure }
}

/// Takes every event the reader has made, folding it into the boxes gathered
fn drain(reader: &mut BoxReader, reported: &mut Vec<Reported>) {
    while let Some(polled) = reader.poll_event() {
        let file_offset = polled.file_offset();

        match polled.into_event() {
            BoxEvent::RawStart(header) => reported.push(Reported::PassedOn {
                header,
                file_offset,
                payload: Vec::new(),
                ended: false,
            }),
            BoxEvent::RawPayload(part) => {
                assert!(!part.is_empty(), "an empty payload event was reported");
                let open = reported.last_mut().expect("a payload before any box started");

                match open {
                    Reported::PassedOn { payload, .. } => payload.extend_from_slice(&part),
                    Reported::Value { .. } => panic!("a payload of a box read into a value"),
                }
            }
            BoxEvent::RawEnd => match reported.last_mut().expect("an end before any box started") {
                Reported::PassedOn { ended, .. } => *ended = true,
                Reported::Value { .. } => panic!("an end of a box read into a value"),
            },
            BoxEvent::FileType(_ftyp) => reported.push(Reported::Value {
                box_type: BoxType::compact(*b"ftyp"),
                file_offset,
            }),
            BoxEvent::SegmentType(_styp) => reported.push(Reported::Value {
                box_type: BoxType::compact(*b"styp"),
                file_offset,
            }),
            BoxEvent::Movie(_moov) => reported.push(Reported::Value {
                box_type: BoxType::compact(*b"moov"),
                file_offset,
            }),
            BoxEvent::MovieFragment(_moof) => reported.push(Reported::Value {
                box_type: BoxType::compact(*b"moof"),
                file_offset,
            }),
            unknown => panic!("the reader reported an event this run cannot check: {unknown:?}"),
        }
    }
}

/// Cuts `bytes` at the cycled `lengths`, each one byte longer than it reads
fn cut_into(bytes: &[u8], lengths: [u8; 4]) -> impl Iterator<Item = &[u8]> {
    let mut rest = bytes;

    lengths.into_iter().cycle().map_while(move |length| {
        if rest.is_empty() {
            return None;
        }
        let (taken, remainder) = rest.split_at((usize::from(length) + 1).min(rest.len()));
        rest = remainder;
        Some(taken)
    })
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
        let (Reported::PassedOn { file_offset, .. } | Reported::Value { file_offset, .. }) =
            *reported;

        assert_eq!(
            file_offset, offset as u64,
            "a box was reported at an offset it does not begin at"
        );

        match reported {
            Reported::PassedOn {
                header, payload, ..
            } => {
                assert_eq!(
                    *header,
                    framed.header(),
                    "the reader and the iterator disagree on the header of a box"
                );
                assert_eq!(
                    payload.as_slice(),
                    framed.payload(),
                    "the reader and the iterator disagree on the payload of a box"
                );
            }
            Reported::Value { box_type, .. } => assert_eq!(
                *box_type,
                framed.header().box_type(),
                "a box was read into a value of another box type"
            ),
        }

        offset += framed.header().encoded_len() + framed.payload().len();
    }
}
