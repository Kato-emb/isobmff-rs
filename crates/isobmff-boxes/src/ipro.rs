//! [`ItemProtectionBox`], the protection schemes the items of a meta box are protected with

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::full_box::{read_version_zero, write_version_zero};

/// [`ItemProtectionBox`] (`ipro`), ISO/IEC 14496-12 §8.11.5
///
/// The protection schemes applied to items, one `ProtectionSchemeInfoBox`
/// each, which the `item_protection_index` of an `ItemInfoEntry` counts from 1.
///
/// The children are kept in [`other_boxes`](Self::other_boxes), in the order
/// they came, and written back in it. A box whose `protection_count` differs
/// from the number of children it holds is refused with
/// [`EntryCountMismatch`](isobmff_core::ErrorKind::EntryCountMismatch), and
/// more children than 16 bits count are refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
/// `protection_count` when the box is written.
#[doc(alias = "ipro")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct ItemProtectionBox {
    /// Protection schemes, in the order the `item_protection_index` counts them
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for ItemProtectionBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"ipro");
}

impl BoxDecode for ItemProtectionBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;
        let protection_count = reader.read_u16()?;
        let other_boxes = OtherBoxes::from(ChildBoxes::decode(reader.take_remainder())?);
        let children = u64::try_from(other_boxes.as_slice().len()).unwrap_or(u64::MAX);
        if u64::from(protection_count) != children {
            return Err(Error::entry_count_mismatch(
                u64::from(protection_count),
                children,
            ));
        }

        Ok(Self { other_boxes })
    }
}

impl BoxEncode for ItemProtectionBox {
    fn payload_len(&self) -> u64 {
        self.other_boxes
            .as_slice()
            .iter()
            .map(AnyBox::encoded_len)
            .fold(4 + 2, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let protection_count = u16::try_from(self.other_boxes.as_slice().len())
            .map_err(|_| Error::invalid_value("protection_count"))?;
        write_version_zero(writer)?;
        writer.write_u16(protection_count)?;
        let mut rest = writer.take_remainder();
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, ChildBoxes, Error, OtherBoxes};

    use super::ItemProtectionBox;
    use crate::encoded;

    /// Two `sinf` holding nothing
    const SINF_TWICE: &[u8] = b"\0\0\0\x08sinf\0\0\0\x08sinf";

    /// `ipro` of the two `sinf`
    const IPRO: &[u8] = b"\0\0\0\x1eipro\0\0\0\0\0\x02\0\0\0\x08sinf\0\0\0\x08sinf";

    fn protection() -> ItemProtectionBox {
        ItemProtectionBox {
            other_boxes: OtherBoxes::from(ChildBoxes::decode(SINF_TWICE).unwrap()),
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            ItemProtectionBox::decode(IPRO),
            Ok((protection(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&protection()), IPRO);
    }

    #[test]
    fn a_count_that_differs_from_the_children_is_refused() {
        assert_eq!(
            ItemProtectionBox::decode_payload(b"\0\0\0\0\0\x03\0\0\0\x08sinf"),
            Err(Error::entry_count_mismatch(3, 1))
        );
    }

    #[test]
    fn a_payload_ending_inside_the_count_is_refused() {
        assert_eq!(
            ItemProtectionBox::decode_payload(b"\0\0\0\0\0"),
            Err(Error::truncated_payload(6, 5))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            ItemProtectionBox::decode_payload(b"\x01\0\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
