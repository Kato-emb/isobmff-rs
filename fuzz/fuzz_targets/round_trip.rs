//! Round-trip properties of [`BoxWriter`] against [`BoxReader`]
//!
//! One run checks four properties of the same input:
//!
//! 1. no call panics: the writer takes every event the reader made of the boxes
//!    it read whole, and finishes the file they make
//! 2. the extents the writer reports partition the file it lays down: each one
//!    covers the bytes its event was written to, and ends where the next one
//!    begins
//! 3. the reader reads the file back rather than rejecting it
//! 4. what it reads back is the events the file was written from
//!
//! Property 4 is the round trip fixed where the input itself cannot fix it: the
//! bytes handed to the reader are not the bytes that come back out — a box read
//! into a value is written behind the header its value settles rather than the
//! one it arrived behind — so it is the second reading that the first is held
//! against, not the input.

#![no_main]

use isobmff_sequence::{BoxEvent, BoxReader, BoxWriter};
use libfuzzer_sys::arbitrary::{self, Arbitrary};
use libfuzzer_sys::fuzz_target;

/// Input of one run: bytes to read, the lengths to cut them into, and the lengths to drain by
///
/// A corpus file is four bytes of `cut_lengths`, four of `drain_lengths`, and
/// then the bytes themselves, verbatim. Both sets of lengths are cycled, each
/// one byte longer than it reads, so a cut and a buffer are 1 to 256 bytes and
/// the run always advances.
#[derive(Arbitrary, Debug)]
struct Input<'bytes> {
    // Why not put `bytes` first, and why a slice rather than a `Vec`: only the
    // last field is taken by `arbitrary_take_rest`, and of the two only
    // `&[u8]` takes the rest verbatim — a `Vec<u8>` reads a byte of its own
    // before each element it keeps, which leaves the seed files unreadable as a
    // hexdump of the input.
    cut_lengths: [u8; 4],
    drain_lengths: [u8; 4],
    bytes: &'bytes [u8],
}

/// Everything one pass of the reader over a file reported
struct Reading {
    events: Vec<BoxEvent>,
    failed: bool,
}

fuzz_target!(|input: Input<'_>| {
    let Input {
        cut_lengths,
        drain_lengths,
        bytes,
    } = input;

    let events = whole_boxes(events_of(cut_into(bytes, cut_lengths)).events);
    let file = file_of(&events, drain_lengths);
    let read_back = events_of([file.as_slice()]);

    assert!(
        !read_back.failed,
        "the reader rejects the file the writer laid down"
    );
    assert_eq!(
        events, read_back.events,
        "the file the writer laid down reads back as other events"
    );
});

/// Hands `arriving` to a reader and gathers the events it reports
///
/// The parts a payload arrives in come back fused: how the input was cut is the
/// caller's and not the file's, and two readings are held against each other
/// without it.
fn events_of<'input>(arriving: impl IntoIterator<Item = &'input [u8]>) -> Reading {
    let mut reader = BoxReader::new();
    let mut events = Vec::new();

    for input in arriving {
        // Why drain before leaving on the failure: the events made before it are
        // still the reader's to hand over, and the boxes whole among them are
        // what the writer is handed.
        let outcome = reader.handle_input(input);
        drain(&mut reader, &mut events);

        if outcome.is_err() {
            return Reading {
                events,
                failed: true,
            };
        }
    }

    let outcome = reader.finish();
    drain(&mut reader, &mut events);

    Reading {
        events,
        failed: outcome.is_err(),
    }
}

/// Takes every event the reader has made, fusing the payload parts of one box
fn drain(reader: &mut BoxReader, events: &mut Vec<BoxEvent>) {
    while let Some(polled) = reader.poll_event() {
        match (events.last_mut(), polled) {
            (Some(BoxEvent::RawPayload(fused)), BoxEvent::RawPayload(part)) => {
                fused.extend_from_slice(&part);
            }
            (_not_two_payload_parts, event) => events.push(event),
        }
    }
}

/// The events up to the last box the reader closed
///
/// A file cut short leaves a box open behind it, and a box left open is not one
/// the writer takes: it would end the file inside that box, which the writer
/// reports as unfinished rather than laying down.
fn whole_boxes(mut events: Vec<BoxEvent>) -> Vec<BoxEvent> {
    let closed = events
        .iter()
        .rposition(|event| !matches!(*event, BoxEvent::RawStart(..) | BoxEvent::RawPayload(..)));

    events.truncate(closed.map_or(0, |last| last + 1));

    events
}

/// Hands `events` to a writer, drained by the cycled `lengths`, and gathers the file it lays down
fn file_of(events: &[BoxEvent], lengths: [u8; 4]) -> Vec<u8> {
    let mut writer = BoxWriter::new();
    let mut buffer_lengths = lengths.into_iter().cycle();
    let mut file = Vec::new();
    let mut covered = 0;

    for event in events {
        writer
            .handle_event(event.clone())
            .expect("the writer rejected an event the reader made");

        let extent = writer
            .event_extent()
            .expect("an event was handed over, so it has an extent");

        assert_eq!(
            extent.start,
            laid_down(&file),
            "an event does not begin where the one before it ended"
        );

        drain_into(&mut writer, &mut buffer_lengths, &mut file);

        assert_eq!(
            extent.end,
            laid_down(&file),
            "the extent of an event is not the bytes it was written to"
        );
        covered = extent.end;
    }

    writer
        .finish()
        .expect("the writer rejected the file it had laid down whole boxes for");
    drain_into(&mut writer, &mut buffer_lengths, &mut file);

    assert_eq!(
        covered,
        laid_down(&file),
        "the extents leave out bytes of the file the writer laid down"
    );

    file
}

/// Takes what the writer has made into `file`, a buffer of the next length at a time
fn drain_into(writer: &mut BoxWriter, lengths: &mut impl Iterator<Item = u8>, file: &mut Vec<u8>) {
    loop {
        let length = lengths.next().expect("the lengths are cycled");
        let mut buffer = vec![0; usize::from(length) + 1];
        let taken = writer.poll_output(&mut buffer);

        match buffer.get(..taken) {
            Some([]) | None => return,
            Some(bytes) => file.extend_from_slice(bytes),
        }
    }
}

/// How far into the file the writer has laid bytes down
fn laid_down(file: &[u8]) -> u64 {
    u64::try_from(file.len()).expect("a file beyond `u64` was laid down")
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
