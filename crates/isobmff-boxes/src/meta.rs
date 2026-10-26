//! [`MetaBox`], the untimed metadata and items of a file, a movie, a track or a fragment

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::dinf::DataInformationBox;
use crate::full_box::{read_version_zero, write_version_zero};
use crate::hdlr::HandlerBox;
use crate::idat::ItemDataBox;
use crate::iinf::ItemInfoBox;
use crate::iloc::ItemLocationBox;
use crate::ipro::ItemProtectionBox;
use crate::iprp::ItemPropertiesBox;
use crate::iref::ItemReferenceBox;
use crate::pitm::PrimaryItemBox;
use crate::xml::XmlVariant;

/// [`MetaBox`] (`meta`), ISO/IEC 14496-12 §8.11.1
///
/// Untimed metadata, and the items of a file: what they are, where their data
/// lies, and which is primary. The box is a container that opens with the
/// fields of a full box.
///
/// Every child is optional, `hdlr` included: the 2026 edition no longer
/// requires it. The box writes `hdlr`, `dinf`, `iloc`, `ipro`, `iinf`, the
/// XML, `pitm`, `idat`, `iref` and `iprp` in this order, whatever order they
/// were read in: the order of ISO/IEC 14496-12 Table 2, and for `iprp`, which
/// Table 2 does not list, right after `iref`, as in the Syntax of the box.
/// Children of other types are kept in
/// [`other_boxes`](Self::other_boxes), and written back after them. A child
/// that does not read is kept there too, and reads back [`None`].
#[doc(alias = "meta")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct MetaBox {
    /// Structure or format of what the box holds, for the items that have no `HandlerProperty`
    pub hdlr: Option<HandlerBox>,
    /// Files in which the data of the items lies
    pub dinf: Option<DataInformationBox>,
    /// Where the data of each item lies
    pub iloc: Option<ItemLocationBox>,
    /// Protection schemes applied to items
    pub ipro: Option<ItemProtectionBox>,
    /// Information about the items
    pub iinf: Option<ItemInfoBox>,
    /// XML stored directly in the box, as text or in a binary form
    pub xml: Option<XmlVariant>,
    /// Primary item
    pub pitm: Option<PrimaryItemBox>,
    /// Data of the items the box stores itself
    pub idat: Option<ItemDataBox>,
    /// Typed references from items to other items
    pub iref: Option<ItemReferenceBox>,
    /// Item properties and the items they are associated with
    pub iprp: Option<ItemPropertiesBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for MetaBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"meta");
}

impl BoxDecode for MetaBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            hdlr: children.take_zero_or_one_if_decoded()?,
            dinf: children.take_zero_or_one_if_decoded()?,
            iloc: children.take_zero_or_one_if_decoded()?,
            ipro: children.take_zero_or_one_if_decoded()?,
            iinf: children.take_zero_or_one_if_decoded()?,
            xml: children.take_zero_or_one_variant_if_decoded()?,
            pitm: children.take_zero_or_one_if_decoded()?,
            idat: children.take_zero_or_one_if_decoded()?,
            iref: children.take_zero_or_one_if_decoded()?,
            iprp: children.take_zero_or_one_if_decoded()?,
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for MetaBox {
    fn payload_len(&self) -> u64 {
        [
            4,
            self.hdlr.as_ref().map_or(0, BoxEncode::encoded_len),
            self.dinf.as_ref().map_or(0, BoxEncode::encoded_len),
            self.iloc.as_ref().map_or(0, BoxEncode::encoded_len),
            self.ipro.as_ref().map_or(0, BoxEncode::encoded_len),
            self.iinf.as_ref().map_or(0, BoxEncode::encoded_len),
            self.xml.as_ref().map_or(0, |xml| match xml {
                XmlVariant::Xml(text) => text.encoded_len(),
                XmlVariant::BinaryXml(binary) => binary.encoded_len(),
            }),
            self.pitm.as_ref().map_or(0, BoxEncode::encoded_len),
            self.idat.as_ref().map_or(0, BoxEncode::encoded_len),
            self.iref.as_ref().map_or(0, BoxEncode::encoded_len),
            self.iprp.as_ref().map_or(0, BoxEncode::encoded_len),
        ]
        .into_iter()
        .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
        .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        let mut rest = writer.take_remainder();
        if let Some(hdlr) = &self.hdlr {
            rest = hdlr.encode(rest)?;
        }
        if let Some(dinf) = &self.dinf {
            rest = dinf.encode(rest)?;
        }
        if let Some(iloc) = &self.iloc {
            rest = iloc.encode(rest)?;
        }
        if let Some(ipro) = &self.ipro {
            rest = ipro.encode(rest)?;
        }
        if let Some(iinf) = &self.iinf {
            rest = iinf.encode(rest)?;
        }
        rest = match &self.xml {
            Some(XmlVariant::Xml(text)) => text.encode(rest)?,
            Some(XmlVariant::BinaryXml(binary)) => binary.encode(rest)?,
            None => rest,
        };
        if let Some(pitm) = &self.pitm {
            rest = pitm.encode(rest)?;
        }
        if let Some(idat) = &self.idat {
            rest = idat.encode(rest)?;
        }
        if let Some(iref) = &self.iref {
            rest = iref.encode(rest)?;
        }
        if let Some(iprp) = &self.iprp {
            rest = iprp.encode(rest)?;
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

    use isobmff_core::{BoxDecode, ChildBoxes, Error, FourCc, OtherBoxes};

    use super::MetaBox;
    use crate::encoded;
    use crate::hdlr::HandlerBox;
    use crate::idat::ItemDataBox;
    use crate::iinf::ItemInfoBox;
    use crate::iloc::{ItemLocation, ItemLocationBox, ItemLocationExtent};
    use crate::ipma::ItemPropertyAssociationBox;
    use crate::ipro::ItemProtectionBox;
    use crate::iprp::ItemPropertiesBox;
    use crate::iref::ItemReferenceBox;
    use crate::pitm::PrimaryItemBox;
    use crate::xml::{BinaryXmlBox, XmlVariant};

    /// `hdlr` of the handler type `test` and an empty name
    const HDLR: &[u8] = b"\0\0\0\x21hdlr\0\0\0\0\0\0\0\0test\0\0\0\0\0\0\0\0\0\0\0\0\0";

    /// `iloc` of version 1 placing item 1 in `idat`, at 0 for its two bytes
    const ILOC: &[u8] =
        b"\0\0\0\x20iloc\x01\0\0\0\x44\0\0\x01\0\x01\0\x01\0\0\0\x01\0\0\0\0\0\0\0\x02";

    /// `bxml` holding one byte
    const BXML: &[u8] = b"\0\0\0\x0dbxml\0\0\0\0\x07";

    /// `pitm` of version 0 naming item 1
    const PITM: &[u8] = b"\0\0\0\x0epitm\0\0\0\0\0\x01";

    /// `idat` of two bytes
    const IDAT: &[u8] = b"\0\0\0\x0aidat\xaa\xbb";

    /// `ipro` of no protection scheme
    const IPRO: &[u8] = b"\0\0\0\x0eipro\0\0\0\0\0\0";

    /// `iinf` of no entry
    const IINF: &[u8] = b"\0\0\0\x0eiinf\0\0\0\0\0\0";

    /// `iref` of version 0 and no reference
    const IREF: &[u8] = b"\0\0\0\x0ciref\0\0\0\0";

    /// `iprp` of an empty `ipco` and an `ipma` of version 0 and no entry
    const IPRP: &[u8] = b"\0\0\0\x20iprp\0\0\0\x08ipco\0\0\0\x10ipma\0\0\0\0\0\0\0\0";

    /// `ipmc` holding nothing, a box this box holds no field for
    const IPMC: &[u8] = b"\0\0\0\x08ipmc";

    fn meta() -> MetaBox {
        MetaBox {
            hdlr: Some(HandlerBox {
                handler_type: FourCc::new(*b"test"),
                ..HandlerBox::default()
            }),
            iloc: Some(ItemLocationBox {
                version: 1,
                offset_size: 4,
                length_size: 4,
                items: vec![ItemLocation {
                    item_id: 1,
                    construction_method: 1,
                    extents: vec![ItemLocationExtent {
                        extent_length: 2,
                        ..ItemLocationExtent::default()
                    }],
                    ..ItemLocation::default()
                }],
                ..ItemLocationBox::default()
            }),
            xml: Some(XmlVariant::BinaryXml(BinaryXmlBox { data: vec![7] })),
            pitm: Some(PrimaryItemBox {
                version: 0,
                item_id: 1,
            }),
            idat: Some(ItemDataBox {
                data: vec![0xaa, 0xbb],
            }),
            ipro: Some(ItemProtectionBox::default()),
            iinf: Some(ItemInfoBox::default()),
            iref: Some(ItemReferenceBox::default()),
            iprp: Some(ItemPropertiesBox {
                ipma: vec![ItemPropertyAssociationBox::default()],
                ..ItemPropertiesBox::default()
            }),
            other_boxes: OtherBoxes::from(ChildBoxes::decode(IPMC).unwrap()),
            ..MetaBox::default()
        }
    }

    #[test]
    fn reads_its_children_in_any_order_and_keeps_the_child_it_has_no_field_for() {
        let bytes = [
            b"\0\0\0\xc2meta\0\0\0\0".as_slice(),
            IPRP,
            IREF,
            IDAT,
            IPMC,
            IINF,
            IPRO,
            PITM,
            BXML,
            ILOC,
            HDLR,
        ]
        .concat();

        assert_eq!(MetaBox::decode(&bytes), Ok((meta(), b"".as_slice())));
    }

    #[test]
    fn writes_hdlr_first_then_the_order_of_table_2_then_the_child_it_has_no_field_for() {
        assert_eq!(
            encoded(&meta()),
            [
                b"\0\0\0\xc2meta\0\0\0\0".as_slice(),
                HDLR,
                ILOC,
                IPRO,
                IINF,
                BXML,
                PITM,
                IDAT,
                IREF,
                IPRP,
                IPMC
            ]
            .concat()
        );
    }

    #[test]
    fn a_box_of_no_child_reads_and_writes_back() {
        let empty = b"\0\0\0\x0cmeta\0\0\0\0";

        assert_eq!(
            MetaBox::decode(empty).map(|(read, _)| encoded(&read)),
            Ok(empty.to_vec())
        );
    }

    #[test]
    fn a_payload_ending_inside_the_full_box_fields_is_refused() {
        assert_eq!(
            MetaBox::decode_payload(b"\0\0"),
            Err(Error::truncated_payload(4, 2))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            MetaBox::decode_payload(b"\x01\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
