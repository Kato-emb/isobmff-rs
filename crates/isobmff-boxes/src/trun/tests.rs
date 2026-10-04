use alloc::vec;
use alloc::vec::Vec;

use isobmff_core::{BoxDecode, BoxEncode, Error};

use super::{CompositionTimeOffset, MAXIMUM_EMPTY_ROWS, TrackRunBox, TrackRunSample};
use crate::SampleFlags;

/// Row stating the size of its sample and the offset to its composition time
fn sample(sample_size: u32, sample_composition_time_offset: i64) -> TrackRunSample {
    TrackRunSample::new(
        None,
        Some(sample_size),
        None,
        Some(CompositionTimeOffset::new(sample_composition_time_offset).unwrap()),
    )
}

/// Run of two samples anchored at the start of the data of its fragment
fn track_run() -> TrackRunBox {
    TrackRunBox::new(Some(0), None, vec![sample(1_024, 0), sample(2_048, 512)]).unwrap()
}

/// Writes the payload of the box and returns the bytes it occupies
fn encoded_payload(track_run: &TrackRunBox) -> Vec<u8> {
    let mut buffer = vec![0; usize::try_from(track_run.payload_len()).unwrap()];
    track_run.encode_payload(&mut buffer).unwrap();

    buffer
}

#[test]
fn a_box_reads_back_as_the_value_that_wrote_it() {
    let payload = encoded_payload(&track_run());

    assert_eq!(TrackRunBox::decode_payload(&payload).unwrap(), track_run());
}

#[test]
fn the_sample_count_is_written_from_the_rows_the_box_holds() {
    let payload = encoded_payload(&track_run());

    assert_eq!(payload.get(4..8), Some(b"\0\0\0\x02".as_slice()));
}

#[test]
fn the_flags_state_which_fields_the_run_and_its_rows_carry() {
    let payload = encoded_payload(&track_run());

    assert_eq!(payload.get(..4), Some(b"\0\0\x0a\x01".as_slice()));
}

#[test]
fn a_run_holding_no_samples_declares_a_count_of_zero() {
    let payload = encoded_payload(&TrackRunBox::new(None, None, Vec::new()).unwrap());

    assert_eq!(payload, b"\0\0\0\0\0\0\0\0");
}

#[test]
fn an_offset_past_the_signed_range_is_written_unsigned_at_version_0() {
    let run = TrackRunBox::new(None, None, vec![sample(1_024, 3_000_000_000)]).unwrap();

    let payload = encoded_payload(&run);

    assert_eq!(payload.first(), Some(&0));
    assert_eq!(TrackRunBox::decode_payload(&payload).unwrap(), run);
}

#[test]
fn a_negative_offset_moves_the_offsets_of_the_run_to_version_1() {
    let run = TrackRunBox::new(None, None, vec![sample(1_024, -512)]).unwrap();

    let payload = encoded_payload(&run);

    assert_eq!(payload.first(), Some(&1));
    assert_eq!(TrackRunBox::decode_payload(&payload).unwrap(), run);
}

#[test]
fn a_run_mixing_a_negative_offset_with_one_past_the_signed_range_cannot_be_built() {
    assert_eq!(
        TrackRunBox::new(
            None,
            None,
            vec![sample(1_024, -512), sample(2_048, 3_000_000_000)]
        ),
        None
    );
}

#[test]
fn a_run_whose_rows_carry_different_fields_cannot_be_built() {
    assert_eq!(
        TrackRunBox::new(
            None,
            None,
            vec![
                sample(1_024, 0),
                TrackRunSample::new(None, Some(2_048), None, None)
            ]
        ),
        None
    );
}

#[test]
fn a_run_stating_the_flags_of_its_first_sample_and_of_every_sample_cannot_be_built() {
    let flagged = TrackRunSample::new(None, None, Some(SampleFlags::ZERO), None);

    assert_eq!(
        TrackRunBox::new(None, Some(SampleFlags::ZERO), vec![flagged]),
        None
    );
}

#[test]
fn a_payload_stating_the_flags_of_its_first_sample_and_of_every_sample_is_rejected() {
    let payload = b"\0\0\x04\x04\0\0\0\x01\0\0\0\0\0\0\0\0";

    assert_eq!(
        TrackRunBox::decode_payload(payload),
        Err(Error::conflicting_flags(0x0000_0404))
    );
}

#[test]
fn first_sample_flags_setting_a_reserved_bit_are_rejected() {
    let payload = b"\0\0\0\x04\0\0\0\x01\x10\0\0\0";

    assert_eq!(
        TrackRunBox::decode_payload(payload),
        Err(Error::unsupported_flags(0x1000_0000))
    );
}

#[test]
fn sample_flags_of_a_row_setting_a_reserved_bit_are_rejected() {
    let payload = b"\0\0\x04\0\0\0\0\x01\x10\0\0\0";

    assert_eq!(
        TrackRunBox::decode_payload(payload),
        Err(Error::unsupported_flags(0x1000_0000))
    );
}

#[test]
fn a_flag_the_box_does_not_read_is_rejected() {
    let payload = b"\0\0\x10\0\0\0\0\0";

    assert_eq!(
        TrackRunBox::decode_payload(payload),
        Err(Error::unsupported_flags(0x0000_1000))
    );
}

#[test]
fn a_declared_count_of_rows_is_weighed_against_the_payload_before_a_row_is_held() {
    /// Bytes the fields of the box require: its own eight, then a row of four
    /// for every sample the count declares
    const NEEDED: u64 = 8 + u32::MAX as u64 * 4;

    let payload = b"\0\0\x01\0\xff\xff\xff\xff";

    assert_eq!(
        TrackRunBox::decode_payload(payload),
        Err(Error::truncated_payload(NEEDED, 8))
    );
}

/// Payload of a run stating no per-sample field, so that its rows are empty
fn empty_rows(sample_count: u32) -> Vec<u8> {
    let mut payload = vec![0; 4];
    payload.extend_from_slice(&sample_count.to_be_bytes());

    payload
}

#[test]
fn a_run_of_empty_rows_is_read_up_to_the_rows_the_box_holds() {
    let sample_count = u32::try_from(MAXIMUM_EMPTY_ROWS).unwrap();

    let run = TrackRunBox::decode_payload(&empty_rows(sample_count)).unwrap();

    assert_eq!(
        run.samples().len(),
        usize::try_from(MAXIMUM_EMPTY_ROWS).unwrap()
    );
}

#[test]
fn a_count_of_empty_rows_past_the_rows_the_box_holds_is_rejected() {
    let past_the_limit = u32::try_from(MAXIMUM_EMPTY_ROWS.saturating_add(1)).unwrap();

    assert_eq!(
        TrackRunBox::decode_payload(&empty_rows(past_the_limit)),
        Err(Error::unsupported_entry_count(
            u64::from(past_the_limit),
            MAXIMUM_EMPTY_ROWS
        ))
    );
}

#[test]
fn a_payload_holding_rows_past_the_count_it_declares_is_rejected() {
    let mut payload = encoded_payload(&track_run());
    payload.extend_from_slice(&[0; 8]);

    assert_eq!(
        TrackRunBox::decode_payload(&payload),
        Err(Error::trailing_payload(28, 36))
    );
}

#[test]
fn a_version_the_box_does_not_read_is_rejected() {
    let mut payload = encoded_payload(&track_run());
    *payload.first_mut().unwrap() = 2;

    assert_eq!(
        TrackRunBox::decode_payload(&payload),
        Err(Error::unsupported_version(2))
    );
}
