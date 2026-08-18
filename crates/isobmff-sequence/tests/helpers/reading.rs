//! A file laid out box by box, and the events a reader reports for it
//!
//! Shared across the integration test binaries with
//! `#[path = "helpers/reading.rs"] mod reading;`.

use isobmff_core::{BoxHeader, BoxType};
use isobmff_sequence::{BoxEvent, BoxReader, BoxReaderError};

/// Lays out the box `header` introduces: the header, then `payload`
pub(crate) fn laid_out(header: BoxHeader, payload: &[u8]) -> Vec<u8> {
    let mut buffer = [0; BoxHeader::MAX_ENCODED_LEN];
    let mut bytes = header.encode(&mut buffer).to_vec();

    bytes.extend_from_slice(payload);

    bytes
}

/// Lays out one whole box: the header that `box_type` and `payload` need, then the payload
pub(crate) fn framed(box_type: BoxType, payload: &[u8]) -> Option<Vec<u8>> {
    let header = BoxHeader::with_payload_len(box_type, u64::try_from(payload.len()).ok()?)?;

    Some(laid_out(header, payload))
}

/// Every event a reader reports for `file`, handed over `cut_length` bytes at a time
pub(crate) fn events_of(file: &[u8], cut_length: usize) -> Result<Vec<BoxEvent>, BoxReaderError> {
    let mut reader = BoxReader::new();
    let mut events = Vec::new();

    for arriving in file.chunks(cut_length) {
        reader.handle_read(arriving)?;
        while let Some(event) = reader.poll_event() {
            events.push(event);
        }
    }
    reader.finish()?;
    while let Some(event) = reader.poll_event() {
        events.push(event);
    }

    Ok(events)
}

/// The events with the payload of each box passed on fused back into one
///
/// What is left is what the file says rather than how it was cut
pub(crate) fn payloads_fused(events: Vec<BoxEvent>) -> Vec<BoxEvent> {
    let mut fused: Vec<BoxEvent> = Vec::new();

    for event in events {
        match (fused.last_mut(), event) {
            (Some(BoxEvent::RawPayload(gathered)), BoxEvent::RawPayload(bytes)) => {
                gathered.extend_from_slice(&bytes);
            }
            (_, event) => fused.push(event),
        }
    }

    fused
}
