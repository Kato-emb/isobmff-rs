//! A value written as a payload reads back as the same value, for every vendor box

#![allow(
    clippy::tests_outside_test_module,
    reason = "an integration test binary ships no items, so its tests are the crate root"
)]

#[path = "helpers/vendor.rs"]
mod vendor;

use isobmff_core::{BoxDecode, BoxEncode, BoxRead, BoxWrite, Error};
use vendor::{ExpiryBox, OpaqueDataBox, SequenceNumberBox, VendorMarkerBox};

/// Writes `value` into a buffer of exactly the payload length it declares
fn encoded(value: &impl BoxEncode) -> Result<Vec<u8>, Error> {
    let mut buffer = vec![0; usize::try_from(value.payload_len()).unwrap_or(usize::MAX)];
    value.encode_payload(&mut buffer)?;

    Ok(buffer)
}

#[test]
fn a_fixed_length_payload_reads_back_as_the_value_that_wrote_it() {
    let value = SequenceNumberBox {
        sequence_number: 0x0102_0304,
    };

    let payload = encoded(&value).unwrap();

    assert_eq!(payload, b"\x01\x02\x03\x04".as_slice());
    assert_eq!(SequenceNumberBox::decode_payload(&payload).unwrap(), value);
}

#[test]
fn a_variable_length_payload_reads_back_as_the_value_that_wrote_it() {
    let value = OpaqueDataBox {
        data: b"vendor data".to_vec(),
    };

    let payload = encoded(&value).unwrap();

    assert_eq!(OpaqueDataBox::decode_payload(&payload).unwrap(), value);
}

#[test]
fn an_empty_payload_reads_back_as_the_value_that_wrote_it() {
    let payload = encoded(&VendorMarkerBox).unwrap();

    assert_eq!(
        VendorMarkerBox::decode_payload(&payload).unwrap(),
        VendorMarkerBox
    );
}

#[test]
fn a_payload_opening_with_version_and_flags_reads_back_as_the_value_that_wrote_it() {
    let value = ExpiryBox::new(0x0102_0304);

    let payload = encoded(&value).unwrap();

    assert_eq!(payload, b"\0\0\0\0\x01\x02\x03\x04".as_slice());
    assert_eq!(ExpiryBox::decode_payload(&payload).unwrap(), value);
}

#[test]
fn a_value_too_wide_for_the_first_version_writes_at_the_version_that_holds_it() {
    let value = ExpiryBox::new(0x1_0000_0000);

    let payload = encoded(&value).unwrap();

    assert_eq!(payload, b"\x01\0\0\0\0\0\0\x01\0\0\0\0".as_slice());
    assert_eq!(ExpiryBox::decode_payload(&payload).unwrap(), value);
}

#[test]
fn a_box_declared_under_a_user_type_reads_back_as_the_value_that_wrote_it() {
    let mut input = vec![0; usize::try_from(VendorMarkerBox.encoded_len()).unwrap()];
    VendorMarkerBox.encode(&mut input).unwrap();

    assert_eq!(
        VendorMarkerBox::decode(&input).unwrap(),
        (VendorMarkerBox, b"".as_slice())
    );
}
