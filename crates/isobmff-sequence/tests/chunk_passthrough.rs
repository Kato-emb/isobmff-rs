//! Boxes a file carries come back out of the reader as they went in, however the file was cut

#![allow(
    clippy::tests_outside_test_module,
    reason = "an integration test binary ships no items, so its tests are the crate root"
)]

use isobmff_core::{BoxHeader, BoxSize, BoxType, Uuid, boxes};
use isobmff_sequence::{BoxEvent, BoxReader, BoxReaderError};

/// User type the vendor box of the synthetic file is declared under
const USER_TYPE: Uuid = Uuid::new([
    0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10,
]);

/// Lays out the box `header` introduces: the header, then `payload`
fn laid_out(header: BoxHeader, payload: &[u8]) -> Vec<u8> {
    let mut buffer = [0; BoxHeader::MAX_ENCODED_LEN];
    let mut bytes = header.encode(&mut buffer).to_vec();

    bytes.extend_from_slice(payload);

    bytes
}

/// Lays out one whole box: the header that `box_type` and `payload` need, then the payload
fn framed(box_type: BoxType, payload: &[u8]) -> Option<Vec<u8>> {
    let header = BoxHeader::with_payload_len(box_type, u64::try_from(payload.len()).ok()?)?;

    Some(laid_out(header, payload))
}

/// A synthetic fragmented file, its last box running to the end of it
fn fragmented_file() -> Option<Vec<u8>> {
    // Why not lay out payloads a box would really carry: every box is passed on
    // as it lies, so what they hold does not bear on what is under test here.
    let unbounded = BoxHeader::new(BoxType::compact(*b"mdat"), BoxSize::ToEndOfFile)?;
    let mut file = framed(BoxType::compact(*b"ftyp"), b"isom\0\0\x02\0isomavc1")?;

    file.extend_from_slice(&framed(BoxType::compact(*b"free"), b"")?);
    file.extend_from_slice(&framed(BoxType::compact(*b"moov"), &[0xa5; 40])?);
    file.extend_from_slice(&framed(BoxType::Extended(USER_TYPE), b"vendor!!")?);
    file.extend_from_slice(&framed(BoxType::compact(*b"moof"), &[0x5a; 24])?);
    file.extend_from_slice(&framed(BoxType::compact(*b"mdat"), &[0x11; 64])?);
    file.extend_from_slice(&laid_out(unbounded, &[0x22; 48]));

    Some(file)
}

/// Every event a reader reports for `file`, handed over in chunks of `chunk_length`
fn events_of(file: &[u8], chunk_length: usize) -> Result<Vec<BoxEvent>, BoxReaderError> {
    let mut reader = BoxReader::new();
    let mut events = Vec::new();

    for chunk in file.chunks(chunk_length) {
        reader.handle_read(chunk)?;
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

/// The events with the payload pieces of each box fused into one
///
/// What is left is what the file says rather than how it was cut
fn payload_pieces_fused(events: Vec<BoxEvent>) -> Vec<BoxEvent> {
    let mut fused: Vec<BoxEvent> = Vec::new();

    for event in events {
        match (fused.last_mut(), event) {
            (Some(BoxEvent::RawChunk(gathered)), BoxEvent::RawChunk(bytes)) => {
                gathered.extend_from_slice(&bytes);
            }
            (_, event) => fused.push(event),
        }
    }

    fused
}

/// Each box the reader reported, as its header and the payload it passed on
fn boxes_reported(events: Vec<BoxEvent>) -> Vec<(BoxHeader, Vec<u8>)> {
    let mut reported: Vec<(BoxHeader, Vec<u8>)> = Vec::new();

    for event in events {
        match event {
            BoxEvent::RawStart { header, .. } => reported.push((header, Vec::new())),
            BoxEvent::RawChunk(bytes) => {
                if let Some((_, payload)) = reported.last_mut() {
                    payload.extend_from_slice(&bytes);
                }
            }
            BoxEvent::RawEnd => {}
            _ => {}
        }
    }

    reported
}

/// Where each box the reader reported said it begins
fn offsets_reported(events: Vec<BoxEvent>) -> Vec<u64> {
    events
        .into_iter()
        .filter_map(|event| match event {
            BoxEvent::RawStart { file_offset, .. } => Some(file_offset),
            BoxEvent::RawChunk(_) | BoxEvent::RawEnd => None,
            _ => None,
        })
        .collect()
}

#[test]
fn the_events_reported_do_not_turn_on_how_the_file_was_cut_into_chunks() {
    let file = fragmented_file().unwrap();
    let whole = payload_pieces_fused(events_of(&file, file.len()).unwrap());

    for chunk_length in 1..=file.len() {
        assert_eq!(
            payload_pieces_fused(events_of(&file, chunk_length).unwrap()),
            whole,
            "chunks of {chunk_length} bytes"
        );
    }
}

#[test]
fn the_boxes_reported_are_the_ones_the_boxes_iterator_splits_out() {
    let file = fragmented_file().unwrap();
    let split = boxes(&file)
        .map(|framed| {
            let framed = framed.unwrap();

            (framed.header(), Vec::from(framed.payload()))
        })
        .collect::<Vec<_>>();

    for chunk_length in 1..=file.len() {
        assert_eq!(
            boxes_reported(events_of(&file, chunk_length).unwrap()),
            split,
            "chunks of {chunk_length} bytes"
        );
    }
}

#[test]
fn the_offset_a_box_carries_is_where_it_begins_in_the_file() {
    let file = fragmented_file().unwrap();
    let mut walked = 0_u64;
    let beginnings = boxes(&file)
        .map(|framed| {
            let framed = framed.unwrap();
            let beginning = walked;
            let header_length = u64::try_from(framed.header().encoded_len()).unwrap();
            let payload_length = u64::try_from(framed.payload().len()).unwrap();

            walked = walked
                .saturating_add(header_length)
                .saturating_add(payload_length);

            beginning
        })
        .collect::<Vec<_>>();

    for chunk_length in 1..=file.len() {
        assert_eq!(
            offsets_reported(events_of(&file, chunk_length).unwrap()),
            beginnings,
            "chunks of {chunk_length} bytes"
        );
    }
}
