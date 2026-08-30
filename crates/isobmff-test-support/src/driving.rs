//! The reader and the writer of the box layer, driven over a file

use alloc::vec;
use alloc::vec::Vec;
use core::ops::Range;

use isobmff_sequence::{BoxEvent, BoxReader, BoxWriter, Error};

/// Every event a reader reports for `file`, handed over `cut_length` bytes at a time
///
/// # Errors
///
/// The failure the reader reports for the file, the events made before it
/// dropped along with it
pub fn events_of(file: &[u8], cut_length: usize) -> Result<Vec<(Range<u64>, BoxEvent)>, Error> {
    let mut reader = BoxReader::new();
    let mut events = Vec::new();

    for arriving in file.chunks(cut_length) {
        reader.handle_input(arriving)?;
        while let Some(event) = polled(&mut reader) {
            events.push(event);
        }
    }
    reader.finish()?;
    while let Some(event) = polled(&mut reader) {
        events.push(event);
    }

    Ok(events)
}

/// The next event the reader reports, with the bytes of the file it was read from
pub fn polled(reader: &mut BoxReader) -> Option<(Range<u64>, BoxEvent)> {
    let event = reader.poll_event()?;
    let extent = reader.event_extent()?;

    Some((extent, event))
}

/// The steps with the payload of each box passed on fused back into one
///
/// What is left is what the file says rather than how it was cut, each step with
/// the bytes of the file it was read from — the fused payload with the extent of
/// the whole run
pub fn payloads_fused(events: Vec<(Range<u64>, BoxEvent)>) -> Vec<(Range<u64>, BoxEvent)> {
    let mut fused: Vec<(Range<u64>, BoxEvent)> = Vec::new();

    for step in events {
        match (fused.last_mut(), step) {
            (
                Some((gathered_extent, BoxEvent::Payload(gathered))),
                (extent, BoxEvent::Payload(bytes)),
            ) => {
                gathered_extent.end = extent.end;
                gathered.extend_from_slice(&bytes);
            }
            (_not_two_payloads, step) => fused.push(step),
        }
    }

    fused
}

/// The file a writer lays down for `events`, drained `buffer_length` bytes at a time
///
/// # Errors
///
/// The failure the writer reports for the events, the bytes made before it
/// dropped along with it
pub fn bytes_of(events: Vec<BoxEvent>, buffer_length: usize) -> Result<Vec<u8>, Error> {
    let mut writer = BoxWriter::new();
    let mut buffer = vec![0; buffer_length];
    let mut file = Vec::new();

    for event in events {
        writer.handle_event(event)?;
        drained_into(&mut writer, &mut buffer, &mut file);
    }
    writer.finish()?;
    drained_into(&mut writer, &mut buffer, &mut file);

    Ok(file)
}

/// Drains what the writer has made so far into `file`, through `buffer`
fn drained_into(writer: &mut BoxWriter, buffer: &mut [u8], file: &mut Vec<u8>) {
    loop {
        let written = writer.poll_output(buffer);

        match buffer.get(..written) {
            Some([]) | None => return,
            Some(bytes) => file.extend_from_slice(bytes),
        }
    }
}
