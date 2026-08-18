//! A file read into events is the file the writer lays back down from them

#![allow(
    clippy::tests_outside_test_module,
    reason = "an integration test binary ships no items, so its tests are the crate root"
)]

#[path = "helpers/sequence.rs"]
pub mod sequence;

use isobmff_sequence::{BoxEvent, BoxEventAt, BoxReaderError};

use sequence::{
    bytes_of, events_of, file_passed_on, file_type, fragmented_file, movie, movie_fragment,
    segment_file, written,
};

/// Every synthetic file the round trip is fixed for
///
/// Between them they carry every box read into a value, a box passed on as it
/// lies under each of the two box types, and a box running to the end of the file
fn every_file() -> Option<Vec<Vec<u8>>> {
    Some(vec![fragmented_file()?, segment_file()?, file_passed_on()?])
}

/// The events the reader reports for `file`, as the writer takes them
fn events_to_write(file: &[u8], cut_length: usize) -> Result<Vec<BoxEvent>, BoxReaderError> {
    Ok(events_of(file, cut_length)?
        .into_iter()
        .map(BoxEventAt::into_event)
        .collect())
}

#[test]
fn a_file_is_written_back_from_its_events_however_the_input_was_cut() {
    for file in every_file().unwrap() {
        for cut_length in 1..=file.len() {
            assert_eq!(
                bytes_of(events_to_write(&file, cut_length).unwrap(), file.len()).unwrap(),
                file,
                "cut every {cut_length} bytes"
            );
        }
    }
}

#[test]
fn a_file_is_written_back_from_its_events_however_the_output_was_drained() {
    for file in every_file().unwrap() {
        let events = events_to_write(&file, file.len()).unwrap();

        for buffer_length in 1..=file.len() {
            assert_eq!(
                bytes_of(events.clone(), buffer_length).unwrap(),
                file,
                "drained {buffer_length} bytes at a time"
            );
        }
    }
}

#[test]
fn a_file_written_back_without_the_boxes_passed_on_holds_the_values_that_are_left() {
    let file = fragmented_file().unwrap();
    let values_only = events_to_write(&file, file.len())
        .unwrap()
        .into_iter()
        .filter(|event| {
            !matches!(
                *event,
                BoxEvent::RawStart(..) | BoxEvent::RawPayload(..) | BoxEvent::RawEnd
            )
        })
        .collect();

    assert_eq!(
        bytes_of(values_only, file.len()).unwrap(),
        [
            written(&file_type()).unwrap(),
            written(&movie().unwrap()).unwrap(),
            written(&movie_fragment().unwrap()).unwrap(),
        ]
        .concat()
    );
}
