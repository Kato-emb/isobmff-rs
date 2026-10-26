//! [`ItemProtectionBox`], the protection schemes the items of a meta box are protected with

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::full_box::{read_version_zero, write_version_zero};
use crate::sinf::ProtectionSchemeInfoBox;

/// [`ItemProtectionBox`] (`ipro`), ISO/IEC 14496-12 §8.11.5
///
/// The protection schemes applied to items, one `ProtectionSchemeInfoBox`
/// each, which the `item_protection_index` of an `ItemInfoEntry` counts from 1.
///
/// The protection schemes are kept in
/// [`protection_information`](Self::protection_information), in the order
/// they came, and written back in it, so the indexes stay as they were; a
/// `sinf` that does not read refuses the box, since keeping it elsewhere
/// would shift the indexes of those after it. Children of other types are
/// kept in [`other_boxes`](Self::other_boxes), and written back after them.
/// A box whose `protection_count` differs from the number of `sinf` it holds
/// is refused with
/// [`EntryCountMismatch`](isobmff_core::ErrorKind::EntryCountMismatch), and
/// more `sinf` than 16 bits count are refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
/// `protection_count` when the box is written.
#[doc(alias = "ipro")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct ItemProtectionBox {
    /// Protection schemes, in the order the `item_protection_index` counts them
    pub protection_information: Vec<ProtectionSchemeInfoBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for ItemProtectionBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"ipro");
}

impl BoxDecode for ItemProtectionBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;
        let protection_count = reader.read_u16()?;
        let mut children = ChildBoxes::decode(reader.take_remainder())?;
        let protection_information: Vec<ProtectionSchemeInfoBox> = children.take_zero_or_more()?;
        let held = u64::try_from(protection_information.len()).unwrap_or(u64::MAX);
        if u64::from(protection_count) != held {
            return Err(Error::entry_count_mismatch(
                u64::from(protection_count),
                held,
            ));
        }

        Ok(Self {
            protection_information,
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for ItemProtectionBox {
    fn payload_len(&self) -> u64 {
        self.protection_information
            .iter()
            .map(BoxEncode::encoded_len)
            .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
            .fold(4 + 2, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let protection_count = u16::try_from(self.protection_information.len())
            .map_err(|_| Error::invalid_value("protection_count"))?;
        write_version_zero(writer)?;
        writer.write_u16(protection_count)?;
        let mut rest = writer.take_remainder();
        for sinf in &self.protection_information {
            rest = sinf.encode(rest)?;
        }
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxDefinition, BoxType, ChildBoxes, Error, FourCc, OtherBoxes};

    use super::ItemProtectionBox;
    use crate::encoded;
    use crate::schm::SchemeTypeBox;
    use crate::sinf::ProtectionSchemeInfoBox;

    /// `sinf` of the scheme `cenc`, as item protection lays it out: with no `frma`
    const SINF_CENC: &[u8] = b"\0\0\0\x1csinf\0\0\0\x14schm\0\0\0\0cenc\0\x01\0\0";

    /// `sinf` of the scheme `cbcs`
    const SINF_CBCS: &[u8] = b"\0\0\0\x1csinf\0\0\0\x14schm\0\0\0\0cbcs\0\x01\0\0";

    /// Header and fields of `ipro` of the two schemes
    const IPRO_HEADER: &[u8] = b"\0\0\0\x46ipro\0\0\0\0\0\x02";

    /// Box of a type `ipro` holds no field for
    const FREE: &[u8] = b"\0\0\0\x08free";

    fn sinf(scheme_type: [u8; 4]) -> ProtectionSchemeInfoBox {
        ProtectionSchemeInfoBox {
            schm: Some(SchemeTypeBox {
                scheme_type: FourCc::new(scheme_type),
                scheme_version: 0x0001_0000,
                scheme_uri: None,
            }),
            ..ProtectionSchemeInfoBox::default()
        }
    }

    fn protection() -> ItemProtectionBox {
        ItemProtectionBox {
            protection_information: vec![sinf(*b"cenc"), sinf(*b"cbcs")],
            other_boxes: OtherBoxes::new(),
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            ItemProtectionBox::decode(&[IPRO_HEADER, SINF_CENC, SINF_CBCS].concat()),
            Ok((protection(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            encoded(&protection()),
            [IPRO_HEADER, SINF_CENC, SINF_CBCS].concat()
        );
    }

    #[test]
    fn counts_only_the_protection_schemes_and_writes_the_other_children_after_them() {
        let read = ItemProtectionBox::decode_payload(
            &[b"\0\0\0\0\0\x02".as_slice(), SINF_CENC, FREE, SINF_CBCS].concat(),
        );
        let expected = ItemProtectionBox {
            other_boxes: OtherBoxes::from(ChildBoxes::decode(FREE).unwrap()),
            ..protection()
        };

        assert_eq!(read.as_ref(), Ok(&expected));
        assert_eq!(
            encoded(&expected),
            [
                b"\0\0\0\x4eipro\0\0\0\0\0\x02".as_slice(),
                SINF_CENC,
                SINF_CBCS,
                FREE
            ]
            .concat()
        );
    }

    #[test]
    fn a_protection_scheme_that_does_not_read_is_refused() {
        let sinf_of_two_schemes =
            b"\0\0\0\x30sinf\0\0\0\x14schm\0\0\0\0cenc\0\x01\0\0\0\0\0\x14schm\0\0\0\0cbcs\0\x01\0\0";

        assert_eq!(
            ItemProtectionBox::decode_payload(
                &[b"\0\0\0\0\0\x01".as_slice(), sinf_of_two_schemes].concat()
            ),
            Err(Error::duplicate_box(BoxType::compact(*b"schm"))
                .in_container(ProtectionSchemeInfoBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_count_that_differs_from_the_protection_schemes_is_refused() {
        assert_eq!(
            ItemProtectionBox::decode_payload(&[b"\0\0\0\0\0\x03".as_slice(), SINF_CENC].concat()),
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
