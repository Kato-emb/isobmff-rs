//! What the payload traits promise about the bytes they refuse: a payload is read
//! whole or not at all, and a buffer is the room the payload asked for

#![allow(
    clippy::tests_outside_test_module,
    reason = "an integration test binary ships no items, so its tests are the crate root"
)]

#[path = "helpers/vendor.rs"]
mod vendor;

use isobmff_core::{BoxDecode, BoxEncode, DecodeError, EncodeError};
use vendor::{ExpiryBox, OpaqueDataBox, SequenceNumberBox, VendorMarkerBox};

#[test]
fn a_payload_ending_inside_a_field_is_rejected_as_truncated() {
    assert!(matches!(
        SequenceNumberBox::decode_payload(b"\0\0\x07"),
        Err(DecodeError::TruncatedPayload {
            needed: 4,
            available: 3
        })
    ));
}

#[test]
fn a_payload_with_bytes_past_the_fields_is_rejected_instead_of_trimmed() {
    assert!(matches!(
        SequenceNumberBox::decode_payload(b"\0\0\0\x07!!"),
        Err(DecodeError::TrailingBytes { remaining: 2 })
    ));
}

#[test]
fn a_box_with_no_fields_rejects_a_payload_that_is_not_empty() {
    assert!(matches!(
        VendorMarkerBox::decode_payload(b"!"),
        Err(DecodeError::TrailingBytes { remaining: 1 })
    ));
}

#[test]
fn a_payload_cut_short_of_the_field_its_version_selects_names_the_length_that_version_needs() {
    assert!(matches!(
        ExpiryBox::decode_payload(b"\x01\0\0\0\0\0\0\x01"),
        Err(DecodeError::TruncatedPayload {
            needed: 12,
            available: 8
        })
    ));
}

#[test]
fn a_buffer_shorter_than_the_declared_payload_is_rejected() {
    let value = SequenceNumberBox { sequence_number: 7 };

    assert_eq!(
        value.encode_payload(&mut [0; 3]),
        Err(EncodeError::BufferLengthMismatch {
            expected: 4,
            actual: 3
        })
    );
}

#[test]
fn a_buffer_sized_for_a_version_other_than_the_declared_one_is_rejected() {
    let value = ExpiryBox::new(0x1_0000_0000);

    assert_eq!(
        value.encode_payload(&mut [0; 8]),
        Err(EncodeError::BufferLengthMismatch {
            expected: 12,
            actual: 8
        })
    );
}

#[test]
fn a_buffer_longer_than_the_declared_payload_is_rejected_as_well() {
    let value = OpaqueDataBox {
        data: b"vendor data".to_vec(),
    };
    let mut buffer = vec![0; 32];

    assert_eq!(
        value.encode_payload(&mut buffer),
        Err(EncodeError::BufferLengthMismatch {
            expected: 11,
            actual: 32
        })
    );
}
