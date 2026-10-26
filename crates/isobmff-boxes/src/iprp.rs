//! [`ItemPropertiesBox`], the item properties of a meta box and the items they are associated with

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::ipco::ItemPropertyContainerBox;
use crate::ipma::ItemPropertyAssociationBox;

/// [`ItemPropertiesBox`] (`iprp`), ISO/IEC 14496-12 §8.11.14
///
/// The item properties of a `MetaBox`, and the associations of items with
/// them.
///
/// The box writes `ipco` and then the `ipma`s, whatever order they were read
/// in: the order of its Syntax. Children of other types are kept in
/// [`other_boxes`](Self::other_boxes), and written back after them, and so is
/// an `ipma` that does not read, such as one of a version it does not read.
///
/// The box holds one `ipco`: a box of none does not read. It holds one or more
/// `ipma`s. A box of none reads, but writing it is refused with
/// [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox), so a
/// box built from [`Default`] is given an `ipma` before it is written.
#[doc(alias = "iprp")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct ItemPropertiesBox {
    /// Item properties, in the order they are indexed
    pub ipco: ItemPropertyContainerBox,
    /// Associations of items with the properties, in the order they came
    pub ipma: Vec<ItemPropertyAssociationBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for ItemPropertiesBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"iprp");
}

impl BoxDecode for ItemPropertiesBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            ipco: children.take_exactly_one()?,
            ipma: children.take_zero_or_more_if_decoded(),
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for ItemPropertiesBox {
    fn payload_len(&self) -> u64 {
        [self.ipco.encoded_len()]
            .into_iter()
            .chain(self.ipma.iter().map(BoxEncode::encoded_len))
            .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
            .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.ipma.is_empty() {
            return Err(Error::missing_mandatory_box(
                ItemPropertyAssociationBox::BOX_TYPE,
            ));
        }

        let mut rest = self.ipco.encode(writer.take_remainder())?;
        for association in &self.ipma {
            rest = association.encode(rest)?;
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

    use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, ChildBoxes, Error, OtherBoxes};

    use super::ItemPropertiesBox;
    use crate::encoded;
    use crate::ipco::ItemPropertyContainerBox;
    use crate::ipma::ItemPropertyAssociationBox;

    /// `ipco` holding nothing
    const IPCO: &[u8] = b"\0\0\0\x08ipco";

    /// `ipma` of version 0 and no entry
    const IPMA: &[u8] = b"\0\0\0\x10ipma\0\0\0\0\0\0\0\0";

    /// `ipma` of a version it does not read
    const IPMA_V2: &[u8] = b"\0\0\0\x10ipma\x02\0\0\0\0\0\0\0";

    fn properties() -> ItemPropertiesBox {
        ItemPropertiesBox {
            ipco: ItemPropertyContainerBox::default(),
            ipma: vec![ItemPropertyAssociationBox::default()],
            other_boxes: OtherBoxes::from(ChildBoxes::decode(IPMA_V2).unwrap()),
        }
    }

    #[test]
    fn reads_its_children_in_any_order_and_keeps_the_ipma_it_does_not_read() {
        let bytes = [b"\0\0\0\x30iprp".as_slice(), IPMA_V2, IPMA, IPCO].concat();

        assert_eq!(
            ItemPropertiesBox::decode(&bytes),
            Ok((properties(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_ipco_then_the_ipmas_then_the_children_it_has_no_field_for() {
        assert_eq!(
            encoded(&properties()),
            [b"\0\0\0\x30iprp".as_slice(), IPCO, IPMA, IPMA_V2].concat()
        );
    }

    #[test]
    fn a_box_of_no_ipco_is_refused() {
        assert_eq!(
            ItemPropertiesBox::decode_payload(IPMA),
            Err(Error::missing_mandatory_box(
                ItemPropertyContainerBox::BOX_TYPE
            ))
        );
    }

    #[test]
    fn a_box_of_no_ipma_reads_but_is_refused_when_written() {
        let (read, _) =
            ItemPropertiesBox::decode(&[b"\0\0\0\x10iprp".as_slice(), IPCO].concat()).unwrap();

        assert_eq!(
            read.encode(&mut [0; 16]),
            Err(Error::missing_mandatory_box(
                ItemPropertyAssociationBox::BOX_TYPE
            ))
        );
    }
}
