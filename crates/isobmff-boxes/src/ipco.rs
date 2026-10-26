//! [`ItemPropertyContainerBox`], the item properties of a meta box in the order they are indexed

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, Boxes, Error, FieldReader, FieldWriter,
};

use crate::brnd::BrandProperty;
use crate::hdlp::HandlerProperty;

/// One child of an [`ItemPropertyContainerBox`]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub enum ItemProperty {
    /// Property of the type `brnd`
    Brand(BrandProperty),
    /// Property of the type `hdlp`
    Handler(HandlerProperty),
    /// Property of any other type, free space, or a child that does not decode, kept as its bytes
    Other(AnyBox),
}

/// [`ItemPropertyContainerBox`] (`ipco`), ISO/IEC 14496-12 §8.11.14
///
/// The item properties of a `MetaBox`, which an `ItemPropertyAssociationBox`
/// associates with items by their index from 1, every child counted, free
/// space included.
///
/// The properties ISO/IEC 14496-12 lays out are read into their types; any
/// other child is kept as its bytes, and so is one that does not decode. The
/// children are kept, and written back, in the order they came, so the
/// indexes stay as they were.
///
/// # Untrusted input
///
/// The box reads each child as it frames it, and allocates room for a child
/// only once it has read it, so what it allocates is bounded by the length of
/// the payload, which the caller sets.
#[doc(alias = "ipco")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct ItemPropertyContainerBox {
    /// Properties, in the order the `property_index` counts them
    pub properties: Vec<ItemProperty>,
}

impl BoxDefinition for ItemPropertyContainerBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"ipco");
}

impl BoxDecode for ItemPropertyContainerBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut properties = Vec::new();
        for child in Boxes::new(reader.take_remainder()) {
            let child = child?;
            let box_type = child.header().box_type();
            let payload = child.payload();
            let read = if box_type == BrandProperty::BOX_TYPE {
                BrandProperty::decode_payload(payload)
                    .map(ItemProperty::Brand)
                    .ok()
            } else if box_type == HandlerProperty::BOX_TYPE {
                HandlerProperty::decode_payload(payload)
                    .map(ItemProperty::Handler)
                    .ok()
            } else {
                None
            };
            properties.push(read.unwrap_or_else(|| {
                ItemProperty::Other(AnyBox::from_raw_bytes(box_type, payload.to_vec()))
            }));
        }

        Ok(Self { properties })
    }
}

impl BoxEncode for ItemPropertyContainerBox {
    fn payload_len(&self) -> u64 {
        self.properties
            .iter()
            .map(|property| match property {
                ItemProperty::Brand(brand) => brand.encoded_len(),
                ItemProperty::Handler(handler) => handler.encoded_len(),
                ItemProperty::Other(other) => other.encoded_len(),
            })
            .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = writer.take_remainder();
        for property in &self.properties {
            rest = match property {
                ItemProperty::Brand(brand) => brand.encode(rest)?,
                ItemProperty::Handler(handler) => handler.encode(rest)?,
                ItemProperty::Other(other) => other.encode(rest)?,
            };
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{AnyBox, BoxDecode, BoxType, Error, FourCc};

    use super::{ItemProperty, ItemPropertyContainerBox};
    use crate::brnd::BrandProperty;
    use crate::encoded;
    use crate::hdlp::HandlerProperty;

    /// `hdlp` of the handler type `pict`
    const HDLP: &[u8] = b"\0\0\0\x10hdlp\0\0\0\0pict";

    /// `free` of no byte
    const FREE: &[u8] = b"\0\0\0\x08free";

    /// `ispe` of 64 by 48 pixels, a property ISO/IEC 14496-12 does not lay out
    const ISPE: &[u8] = b"\0\0\0\x14ispe\0\0\0\0\0\0\0\x40\0\0\0\x30";

    /// `brnd` of major brand `heic` and no compatible brand
    const BRND: &[u8] = b"\0\0\0\x10brndheic\0\0\0\0";

    /// `hdlp` of a version it does not read
    const HDLP_V1: &[u8] = b"\0\0\0\x10hdlp\x01\0\0\0pict";

    fn properties() -> ItemPropertyContainerBox {
        ItemPropertyContainerBox {
            properties: vec![
                ItemProperty::Handler(HandlerProperty {
                    handler_type: FourCc::new(*b"pict"),
                }),
                ItemProperty::Other(AnyBox::from_raw_bytes(BoxType::compact(*b"free"), vec![])),
                ItemProperty::Other(AnyBox::from_raw_bytes(
                    BoxType::compact(*b"ispe"),
                    b"\0\0\0\0\0\0\0\x40\0\0\0\x30".to_vec(),
                )),
                ItemProperty::Brand(BrandProperty {
                    major_brand: FourCc::new(*b"heic"),
                    ..BrandProperty::default()
                }),
                ItemProperty::Other(AnyBox::from_raw_bytes(
                    BoxType::compact(*b"hdlp"),
                    b"\x01\0\0\0pict".to_vec(),
                )),
            ],
        }
    }

    #[test]
    fn reads_every_child_in_the_order_it_came_and_keeps_the_ones_it_does_not_lay_out() {
        let bytes = [
            b"\0\0\0\x54ipco".as_slice(),
            HDLP,
            FREE,
            ISPE,
            BRND,
            HDLP_V1,
        ]
        .concat();

        assert_eq!(
            ItemPropertyContainerBox::decode(&bytes),
            Ok((properties(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_every_child_in_the_order_it_holds_them() {
        assert_eq!(
            encoded(&properties()),
            [
                b"\0\0\0\x54ipco".as_slice(),
                HDLP,
                FREE,
                ISPE,
                BRND,
                HDLP_V1
            ]
            .concat()
        );
    }

    #[test]
    fn a_child_cut_short_by_the_payload_is_refused() {
        assert_eq!(
            ItemPropertyContainerBox::decode_payload(b"\0\0\0\x10hdlp\0\0"),
            Err(Error::truncated_box(16, 10))
        );
    }
}
