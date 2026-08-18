//! Boxes passed on as they lie come back out of the reader as they went in, however the file was cut

#![allow(
    clippy::tests_outside_test_module,
    reason = "an integration test binary ships no items, so its tests are the crate root"
)]

#[path = "helpers/reading.rs"]
mod reading;

use isobmff_core::{BoxHeader, BoxSize, BoxType, Uuid, boxes};
use isobmff_sequence::BoxEvent;

use reading::{events_of, framed, laid_out, payloads_fused};

/// User type the vendor box of the synthetic file is declared under
const USER_TYPE: Uuid = Uuid::new([
    0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10,
]);

/// A synthetic file of boxes passed on as they lie, its last box running to the end of it
fn file_passed_on() -> Option<Vec<u8>> {
    // Why hold no box the reader reads into a value: such a box is reported as
    // that value and never as the bytes it was read from, which is what this file
    // fixes. `value_boxes.rs` covers those.
    let unbounded = BoxHeader::new(BoxType::compact(*b"mdat"), BoxSize::ToEndOfFile)?;
    let mut file = framed(BoxType::compact(*b"free"), b"")?;

    file.extend_from_slice(&framed(BoxType::compact(*b"skip"), &[0xa5; 40])?);
    file.extend_from_slice(&framed(BoxType::Extended(USER_TYPE), b"vendor!!")?);
    file.extend_from_slice(&framed(BoxType::compact(*b"mdat"), &[0x11; 64])?);
    file.extend_from_slice(&laid_out(unbounded, &[0x22; 48]));

    Some(file)
}

/// Each box the reader reported, as its header and the payload it passed on
fn boxes_reported(events: &[BoxEvent]) -> Vec<(BoxHeader, Vec<u8>)> {
    let mut reported: Vec<(BoxHeader, Vec<u8>)> = Vec::new();

    for event in events {
        if let BoxEvent::RawStart { header, .. } = *event {
            reported.push((header, Vec::new()));
        } else if let BoxEvent::RawPayload(ref bytes) = *event {
            if let Some((_, payload)) = reported.last_mut() {
                payload.extend_from_slice(bytes);
            }
        }
    }

    reported
}

/// Where each box the reader reported said it begins
fn offsets_reported(events: &[BoxEvent]) -> Vec<u64> {
    events
        .iter()
        .filter_map(|event| {
            if let BoxEvent::RawStart { file_offset, .. } = *event {
                Some(file_offset)
            } else {
                None
            }
        })
        .collect()
}

#[test]
fn the_events_reported_do_not_turn_on_where_the_file_was_cut() {
    let file = file_passed_on().unwrap();
    let whole = payloads_fused(events_of(&file, file.len()).unwrap());

    for cut_length in 1..=file.len() {
        assert_eq!(
            payloads_fused(events_of(&file, cut_length).unwrap()),
            whole,
            "cut every {cut_length} bytes"
        );
    }
}

#[test]
fn the_boxes_reported_are_the_ones_the_boxes_iterator_splits_out() {
    let file = file_passed_on().unwrap();
    let split = boxes(&file)
        .map(|framed| {
            let framed = framed.unwrap();

            (framed.header(), Vec::from(framed.payload()))
        })
        .collect::<Vec<_>>();

    for cut_length in 1..=file.len() {
        assert_eq!(
            boxes_reported(&events_of(&file, cut_length).unwrap()),
            split,
            "cut every {cut_length} bytes"
        );
    }
}

#[test]
fn the_offset_a_box_carries_is_where_it_begins_in_the_file() {
    let file = file_passed_on().unwrap();
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

    for cut_length in 1..=file.len() {
        assert_eq!(
            offsets_reported(&events_of(&file, cut_length).unwrap()),
            beginnings,
            "cut every {cut_length} bytes"
        );
    }
}
