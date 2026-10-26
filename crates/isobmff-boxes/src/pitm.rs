//! [`PrimaryItemBox`], the primary item of a meta box

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFlags,
    FullBoxHeader,
};

use crate::full_box::{read_u16_or_u32, write_u16_or_u32};

/// [`PrimaryItemBox`] (`pitm`), ISO/IEC 14496-12 §8.11.4
///
/// Names the primary item of a `MetaBox`.
///
/// The box is written in the `version` it holds: version 0 carries `item_ID`
/// in 16 bits, version 1 in 32. A box read from a file keeps the version it was
/// read in, and an `item_ID` that does not fit version 0 is refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for `item_ID` when
/// the box is written.
#[doc(alias = "pitm")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct PrimaryItemBox {
    /// Version of the box, 0 or 1, which sets the width of `item_ID`
    pub version: u8,
    /// `item_ID` of the primary item, an item of the `MetaBox` that holds the box
    pub item_id: u32,
}

impl BoxDefinition for PrimaryItemBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"pitm");
}

impl BoxDecode for PrimaryItemBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?).version();
        if version > 1 {
            return Err(Error::unsupported_version(version));
        }

        Ok(Self {
            version,
            item_id: read_u16_or_u32(reader, version == 1)?,
        })
    }
}

impl BoxEncode for PrimaryItemBox {
    fn payload_len(&self) -> u64 {
        if self.version == 1 { 4 + 4 } else { 4 + 2 }
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.version > 1 {
            return Err(Error::unsupported_version(self.version));
        }
        writer.write_bytes(&FullBoxHeader::new(self.version, FullBoxFlags::ZERO).to_bytes())?;
        write_u16_or_u32(
            writer,
            self.version == 1,
            u64::from(self.item_id),
            "item_ID",
        )
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::PrimaryItemBox;
    use crate::encoded;

    /// `pitm` of version 0 naming item 1
    const PITM_V0: &[u8] = b"\0\0\0\x0epitm\0\0\0\0\0\x01";

    /// `pitm` of version 1 naming item 2^16
    const PITM_V1: &[u8] = b"\0\0\0\x10pitm\x01\0\0\0\0\x01\0\0";

    fn primary_item(version: u8, item_id: u32) -> PrimaryItemBox {
        PrimaryItemBox { version, item_id }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_in_either_version() {
        assert_eq!(
            (
                PrimaryItemBox::decode(PITM_V0),
                PrimaryItemBox::decode(PITM_V1)
            ),
            (
                Ok((primary_item(0, 1), b"".as_slice())),
                Ok((primary_item(1, 1 << 16), b"".as_slice()))
            )
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_the_version_it_holds() {
        assert_eq!(
            (
                encoded(&primary_item(0, 1)),
                encoded(&primary_item(1, 1 << 16))
            ),
            (PITM_V0.to_vec(), PITM_V1.to_vec())
        );
    }

    #[test]
    fn version_0_refuses_an_item_id_wider_than_16_bits() {
        assert_eq!(
            primary_item(0, 1 << 16).encode(&mut [0; 14]),
            Err(Error::invalid_value("item_ID"))
        );
    }

    #[test]
    fn a_payload_ending_inside_the_item_id_is_refused() {
        assert_eq!(
            PrimaryItemBox::decode_payload(b"\x01\0\0\0\0\x01"),
            Err(Error::truncated_payload(8, 6))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused_when_read_and_when_written() {
        assert_eq!(
            (
                PrimaryItemBox::decode_payload(b"\x02\0\0\0"),
                primary_item(2, 1).encode(&mut [0; 16])
            ),
            (
                Err(Error::unsupported_version(2)),
                Err(Error::unsupported_version(2))
            )
        );
    }
}
