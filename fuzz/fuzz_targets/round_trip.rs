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
//! Property 4 is the round trip fixed where the input itself cannot fix it: an
//! input cut short leaves a box open, which the writer never lays down, so the
//! file is what the whole boxes make of it rather than the input — and it is the
//! second reading that the first is held against, not the bytes handed over.

#![no_main]

use isobmff_sequence::{BoxEvent, BoxReader, BoxWriter};
use libfuzzer_sys::arbitrary::{self, Arbitrary};
use libfuzzer_sys::fuzz_target;

#[path = "helpers/cut.rs"]
mod cut;

use cut::cut_into;

/// Input of one run: bytes to read, the lengths to cut them into, and the lengths to drain by
///
/// A corpus file is four bytes of `cut_lengths`, four of `drain_lengths`, and
/// then the bytes themselves, verbatim. Both sets of lengths are cycled, each
/// one byte longer than it reads, so a cut and a buffer are 1 to 256 bytes and
/// the run always advances.
#[derive(Arbitrary, Debug)]
struct Input<'bytes> {
    // Why not put `bytes` first, and why a slice: only the last field is handed
    // what is left, and only `&[u8]` takes it verbatim — a `Vec<u8>` reads a
    // byte of its own before each element, so a seed stops at its first even
    // byte.
    cut_lengths: [u8; 4],
    drain_lengths: [u8; 4],
    bytes: &'bytes [u8],
}

/// Longest buffer a drain offers, the widest a byte of `drain_lengths` reads as
const MAX_BUFFER_LEN: usize = 256;

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
    let mut failed = false;

    for input in arriving {
        // Why drain before leaving on the failure: the events made before it are
        // still the reader's to hand over, and the boxes whole among them are
        // what the writer is handed.
        let outcome = reader.handle_input(input);
        drain(&mut reader, &mut events);

        if outcome.is_err() {
            failed = true;
            break;
        }
    }

    if !failed {
        let outcome = reader.finish();
        drain(&mut reader, &mut events);

        failed = outcome.is_err();
    }

    Reading { events, failed }
}

/// Takes every event the reader has made, fusing the payload parts of one box
fn drain(reader: &mut BoxReader, events: &mut Vec<BoxEvent>) {
    while let Some(polled) = reader.poll_event() {
        match (events.last_mut(), polled) {
            (Some(BoxEvent::Payload(fused)), BoxEvent::Payload(part)) => {
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
    while matches!(
        events.last(),
        Some(BoxEvent::Header(..) | BoxEvent::Payload(..))
    ) {
        events.pop();
    }

    events
}

/// Hands `events` to a writer, drained by the cycled `lengths`, and gathers the file it lays down
fn file_of(events: &[BoxEvent], lengths: [u8; 4]) -> Vec<u8> {
    let mut writer = BoxWriter::new();
    let mut buffer_lengths = lengths.into_iter().cycle();
    let mut buffer = [0; MAX_BUFFER_LEN];
    let mut file = Vec::new();

    for event in events {
        writer
            .handle_event(event.clone())
            .expect("the writer rejected an event the reader made");

        let extent = writer
            .event_extent()
            .expect("an event was handed over, so it has an extent");
        let began_at = laid_down(&file);

        drain_into(&mut writer, &mut buffer_lengths, &mut buffer, &mut file);

        assert_eq!(
            extent,
            began_at..laid_down(&file),
            "the extent of an event is not the bytes it was written to"
        );
    }

    let laid_down_by_the_events = laid_down(&file);

    writer
        .finish()
        .expect("the writer rejected the file it had laid down whole boxes for");
    drain_into(&mut writer, &mut buffer_lengths, &mut buffer, &mut file);

    assert_eq!(
        laid_down_by_the_events,
        laid_down(&file),
        "the extents leave out bytes of the file the writer laid down"
    );

    file
}

/// Takes what the writer has made into `file`, a buffer of the next length at a time
fn drain_into(
    writer: &mut BoxWriter,
    lengths: &mut impl Iterator<Item = u8>,
    buffer: &mut [u8; MAX_BUFFER_LEN],
    file: &mut Vec<u8>,
) {
    loop {
        let length = lengths.next().expect("the lengths are cycled");
        let offered = &mut buffer[..usize::from(length) + 1];
        let taken = writer.poll_output(offered);

        if taken == 0 {
            return;
        }
        file.extend_from_slice(&offered[..taken]);
    }
}

/// How far into the file the writer has laid bytes down
fn laid_down(file: &[u8]) -> u64 {
    u64::try_from(file.len()).expect("a file beyond `u64` was laid down")
}
