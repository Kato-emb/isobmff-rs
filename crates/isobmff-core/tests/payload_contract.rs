//! What the payload traits promise about the bytes they refuse: a payload is read
//! whole or not at all, and a buffer is the room the payload asked for

#![allow(
    clippy::tests_outside_test_module,
    reason = "an integration test binary ships no items, so its tests are the crate root"
)]

#[path = "helpers/vendor.rs"]
mod vendor;

use isobmff_core::{BoxDecode, BoxEncode, Error, FieldWriter};
use vendor::{ExpiryBox, OpaqueDataBox, SequenceNumberBox, VendorMarkerBox};

/// Vendor box whose declared length and whose fields disagree on purpose
///
/// A box states its payload length apart from the fields it writes, so the two
/// can differ; this one differs by the length it is built with, which is what
/// the payload traits hold a box to.
struct MisdeclaredBox {
    declared_len: u64,
}

impl BoxEncode for MisdeclaredBox {
    fn payload_len(&self) -> u64 {
        self.declared_len
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(7)
    }
}

#[test]
fn a_payload_ending_inside_a_field_is_rejected_as_truncated() {
    assert_eq!(
        SequenceNumberBox::decode_payload(b"\0\0\x07"),
        Err(Error::truncated_payload(4, 3))
    );
}

#[test]
fn a_payload_with_bytes_past_the_fields_is_rejected_instead_of_trimmed() {
    assert_eq!(
        SequenceNumberBox::decode_payload(b"\0\0\0\x07!!"),
        Err(Error::trailing_payload(4, 6))
    );
}

#[test]
fn a_box_with_no_fields_rejects_a_payload_that_is_not_empty() {
    assert_eq!(
        VendorMarkerBox::decode_payload(b"!"),
        Err(Error::trailing_payload(0, 1))
    );
}

#[test]
fn a_payload_cut_short_of_the_field_its_version_selects_names_the_length_that_version_needs() {
    assert_eq!(
        ExpiryBox::decode_payload(b"\x01\0\0\0\0\0\0\x01"),
        Err(Error::truncated_payload(12, 8))
    );
}

#[test]
fn a_buffer_shorter_than_the_declared_payload_is_rejected() {
    let value = SequenceNumberBox { sequence_number: 7 };

    assert_eq!(
        value.encode_payload(&mut [0; 3]),
        Err(Error::buffer_length_mismatch(4, 3))
    );
}

#[test]
fn a_buffer_sized_for_a_version_other_than_the_declared_one_is_rejected() {
    let value = ExpiryBox::new(0x1_0000_0000);

    assert_eq!(
        value.encode_payload(&mut [0; 8]),
        Err(Error::buffer_length_mismatch(12, 8))
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
        Err(Error::buffer_length_mismatch(11, 32))
    );
}

#[test]
fn a_box_declaring_more_than_its_fields_write_leaves_the_buffer_unfilled() {
    let value = MisdeclaredBox { declared_len: 8 };

    assert_eq!(
        value.encode_payload(&mut [0; 8]),
        Err(Error::trailing_buffer(4, 8))
    );
}

#[test]
fn a_box_declaring_less_than_its_fields_write_runs_out_of_buffer() {
    let value = MisdeclaredBox { declared_len: 2 };

    assert_eq!(
        value.encode_payload(&mut [0; 2]),
        Err(Error::truncated_buffer(4, 2))
    );
}
