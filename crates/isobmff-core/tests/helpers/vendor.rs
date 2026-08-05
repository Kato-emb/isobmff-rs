//! Vendor boxes written against the public API alone, as a third party writing
//! its own boxes would have to
//!
//! Shared across the integration test binaries with
//! `#[path = "helpers/vendor.rs"] mod vendor;`.

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, DecodeError, EncodeError, Uuid};

/// Returns the length as the payload traits count it
fn byte_count(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}

/// Returns the mismatch for a buffer that is not the room the payload asked for
fn buffer_length_mismatch(expected: u64, buffer: &[u8]) -> EncodeError {
    EncodeError::BufferLengthMismatch {
        expected,
        actual: byte_count(buffer.len()),
    }
}

/// Vendor box whose payload is one 32-bit sequence number
#[derive(PartialEq, Eq, Debug)]
pub(crate) struct SequenceNumberBox {
    pub(crate) sequence_number: u32,
}

impl BoxDefinition for SequenceNumberBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"sqnc");
}

impl BoxDecode for SequenceNumberBox {
    fn decode_payload(payload: &[u8]) -> Result<Self, DecodeError> {
        let (field, rest) =
            payload
                .split_first_chunk::<4>()
                .ok_or(DecodeError::TruncatedPayload {
                    needed: 4,
                    available: byte_count(payload.len()),
                })?;

        if !rest.is_empty() {
            return Err(DecodeError::TrailingBytes {
                remaining: byte_count(rest.len()),
            });
        }

        Ok(Self {
            sequence_number: u32::from_be_bytes(*field),
        })
    }
}

impl BoxEncode for SequenceNumberBox {
    fn payload_len(&self) -> u64 {
        4
    }

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), EncodeError> {
        let mismatch = buffer_length_mismatch(self.payload_len(), buffer);
        if byte_count(buffer.len()) != self.payload_len() {
            return Err(mismatch);
        }

        let field = buffer.first_chunk_mut::<4>().ok_or(mismatch)?;
        *field = self.sequence_number.to_be_bytes();

        Ok(())
    }
}

/// Vendor box whose payload is opaque data of any length
#[derive(PartialEq, Eq, Debug)]
pub(crate) struct OpaqueDataBox {
    pub(crate) data: Vec<u8>,
}

impl BoxDefinition for OpaqueDataBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"vdat");
}

impl BoxDecode for OpaqueDataBox {
    fn decode_payload(payload: &[u8]) -> Result<Self, DecodeError> {
        Ok(Self {
            data: payload.to_vec(),
        })
    }
}

impl BoxEncode for OpaqueDataBox {
    fn payload_len(&self) -> u64 {
        byte_count(self.data.len())
    }

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), EncodeError> {
        if buffer.len() != self.data.len() {
            return Err(buffer_length_mismatch(self.payload_len(), buffer));
        }

        buffer.copy_from_slice(&self.data);

        Ok(())
    }
}

/// `usertype` of the vendor box named by a UUID
const VENDOR_UUID: Uuid = Uuid::new([
    0x2b, 0x0d, 0x1a, 0x7f, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc,
]);

/// Vendor box named by a UUID, marking its presence and carrying no payload
#[derive(PartialEq, Eq, Debug)]
pub(crate) struct VendorMarkerBox;

impl BoxDefinition for VendorMarkerBox {
    const BOX_TYPE: BoxType = BoxType::Extended(VENDOR_UUID);
}

impl BoxDecode for VendorMarkerBox {
    fn decode_payload(payload: &[u8]) -> Result<Self, DecodeError> {
        if !payload.is_empty() {
            return Err(DecodeError::TrailingBytes {
                remaining: byte_count(payload.len()),
            });
        }

        Ok(Self)
    }
}

impl BoxEncode for VendorMarkerBox {
    fn payload_len(&self) -> u64 {
        0
    }

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), EncodeError> {
        if !buffer.is_empty() {
            return Err(buffer_length_mismatch(self.payload_len(), buffer));
        }

        Ok(())
    }
}
