//! Boxes passed on as they lie come back out of the reader as they went in, however the file was cut

#![allow(
    clippy::tests_outside_test_module,
    reason = "an integration test binary ships no items, so its tests are the crate root"
)]

#[path = "helpers/sequence.rs"]
pub mod sequence;

use isobmff_core::{BoxHeader, boxes};
use isobmff_sequence::{BoxEvent, BoxEventAt};

use sequence::{events_of, file_passed_on, payloads_fused};

/// Each box the reader reported, as its header and the payload it passed on
fn boxes_reported(events: &[BoxEventAt]) -> Vec<(BoxHeader, Vec<u8>)> {
    let mut reported: Vec<(BoxHeader, Vec<u8>)> = Vec::new();

    for event in events {
        if let BoxEvent::RawStart(header) = *event.event() {
            reported.push((header, Vec::new()));
        } else if let BoxEvent::RawPayload(ref bytes) = *event.event() {
            if let Some((_header, payload)) = reported.last_mut() {
                payload.extend_from_slice(bytes);
            }
        }
    }

    reported
}

/// Where each box the reader reported said it begins
fn offsets_reported(events: &[BoxEventAt]) -> Vec<u64> {
    let mut offsets = Vec::new();

    for event in events {
        if let BoxEvent::RawStart(_header) = *event.event() {
            offsets.push(event.file_offset());
        }
    }

    offsets
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
