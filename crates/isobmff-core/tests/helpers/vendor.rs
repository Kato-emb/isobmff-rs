//! Vendor boxes written against the public API alone, as a third party writing
//! its own boxes would have to
//!
//! Shared across the integration test binaries with
//! `#[path = "helpers/vendor.rs"] mod vendor;`.

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWidth, FieldWriter,
    FullBoxFields, FullBoxFlags, Uuid,
};

/// Returns the length as the payload traits count it
fn byte_count(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}

/// Returns the mismatch for a buffer that is not the room the payload asked for
fn buffer_length_mismatch(expected: u64, buffer: &[u8]) -> Error {
    Error::buffer_length_mismatch(expected, byte_count(buffer.len()))
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
    fn decode_payload(payload: &[u8]) -> Result<Self, Error> {
        let mut reader = FieldReader::new(payload);
        let sequence_number = reader.read_u32()?;
        reader.finish()?;

        Ok(Self { sequence_number })
    }
}

impl BoxEncode for SequenceNumberBox {
    fn payload_len(&self) -> u64 {
        4
    }

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), Error> {
        if byte_count(buffer.len()) != self.payload_len() {
            return Err(buffer_length_mismatch(self.payload_len(), buffer));
        }

        FieldWriter::new(buffer).write_u32(self.sequence_number)?;

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
    fn decode_payload(payload: &[u8]) -> Result<Self, Error> {
        Ok(Self {
            data: payload.to_vec(),
        })
    }
}

impl BoxEncode for OpaqueDataBox {
    fn payload_len(&self) -> u64 {
        byte_count(self.data.len())
    }

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), Error> {
        if buffer.len() != self.data.len() {
            return Err(buffer_length_mismatch(self.payload_len(), buffer));
        }

        buffer.copy_from_slice(&self.data);

        Ok(())
    }
}

/// Vendor full box whose expiry time widens with the version the box declares
///
/// Version 0 carries the time in 32 bits and version 1 in 64, as the boxes that
/// widen with their version do. A version past those reads as version 1.
#[derive(PartialEq, Eq, Debug)]
pub(crate) struct ExpiryBox {
    full_box: FullBoxFields,
    expiry_time: u64,
}

impl ExpiryBox {
    /// Returns the box carrying `expiry_time` at the narrowest version that holds it
    pub(crate) fn new(expiry_time: u64) -> Self {
        let version = if expiry_time > u64::from(u32::MAX) {
            1
        } else {
            0
        };

        // Why not public fields: the version and the time would be set apart,
        // and a box declaring version 0 while holding a time past 32 bits
        // cannot be written at the version it declares.
        Self {
            full_box: FullBoxFields::new(version, FullBoxFlags::ZERO),
            expiry_time,
        }
    }

    /// Returns the width the given version carries the expiry time at
    fn field_width(version: u8) -> FieldWidth {
        match version {
            0 => FieldWidth::Compact,
            _ => FieldWidth::Extended,
        }
    }

    /// Returns the payload length the version selects
    fn payload_len_at_version(version: u8) -> u64 {
        match version {
            0 => 8,
            _ => 12,
        }
    }
}

impl BoxDefinition for ExpiryBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"expy");
}

impl BoxDecode for ExpiryBox {
    fn decode_payload(payload: &[u8]) -> Result<Self, Error> {
        let mut reader = FieldReader::new(payload);
        let full_box = FullBoxFields::from_bytes(reader.read_bytes::<4>()?);
        let expiry_time = reader.read_unsigned(Self::field_width(full_box.version()))?;
        reader.finish()?;

        Ok(Self {
            full_box,
            expiry_time,
        })
    }
}

impl BoxEncode for ExpiryBox {
    fn payload_len(&self) -> u64 {
        Self::payload_len_at_version(self.full_box.version())
    }

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), Error> {
        if byte_count(buffer.len()) != self.payload_len() {
            return Err(buffer_length_mismatch(self.payload_len(), buffer));
        }

        let mut writer = FieldWriter::new(buffer);
        writer.write_bytes(&self.full_box.to_bytes())?;
        writer.write_unsigned(Self::field_width(self.full_box.version()), self.expiry_time)?;

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
    fn decode_payload(payload: &[u8]) -> Result<Self, Error> {
        FieldReader::new(payload).finish()?;

        Ok(Self)
    }
}

impl BoxEncode for VendorMarkerBox {
    fn payload_len(&self) -> u64 {
        0
    }

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), Error> {
        if !buffer.is_empty() {
            return Err(buffer_length_mismatch(self.payload_len(), buffer));
        }

        Ok(())
    }
}
