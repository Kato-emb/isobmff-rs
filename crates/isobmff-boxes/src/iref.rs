//! [`ItemReferenceBox`], the typed references from items to other items

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, Boxes, Error, FieldReader, FieldWriter,
    FourCc, FullBoxFlags, FullBoxHeader,
};

use crate::box_type_argument;
use crate::full_box::{read_u16_or_u32, write_u16_or_u32};
use crate::table::{read_entries, table_len};
use crate::tref::has_adjacent_equal;

/// [`SingleItemTypeReferenceBox`], ISO/IEC 14496-12 §8.11.12
///
/// The references of one type from one item to other items. The type is the
/// box type of the box, which [`ItemReferenceBox`] reads into
/// [`reference_type`](Self::reference_type) and writes from it, so a box of
/// any type, registered or not, reads.
///
/// The `item_ID`s take 16 bits in an [`ItemReferenceBox`] of version 0, and
/// 32 in one of version 1, the `SingleItemTypeReferenceBoxLarge`. The box has
/// no box type of its own and no width of its own, so it implements none of
/// [`BoxDecode`], [`BoxDefinition`] and [`BoxEncode`]: the
/// [`ItemReferenceBox`] reads and writes it.
#[doc(alias = "SingleItemTypeReferenceBoxLarge")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SingleItemTypeReferenceBox {
    /// Type of the references, such as `dimg`, `thmb` or `cdsc`
    ///
    /// It is written as the box type, so a `reference_type` of `uuid` is
    /// refused with [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue)
    /// for `referenceType` when the box is written.
    pub reference_type: FourCc,
    /// `item_ID` of the item that refers to the others
    pub from_item_id: u32,
    /// `item_ID` of each item referred to, which other structures index from 1
    ///
    /// A value given twice is read as found, but refused with
    /// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
    /// `to_item_ID` when the box is written, and so are more values than 16
    /// bits count, for `reference_count`.
    pub to_item_ids: Vec<u32>,
}

impl Default for SingleItemTypeReferenceBox {
    fn default() -> Self {
        Self {
            reference_type: FourCc::new([0; 4]),
            from_item_id: 0,
            to_item_ids: Vec::new(),
        }
    }
}

impl SingleItemTypeReferenceBox {
    /// Reads the payload of a box of the box type `reference_type`, its `item_ID`s 32 bits wide where `large`
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends inside a field, or holds fewer `to_item_ID`s than
    ///   `reference_count`.
    fn decode_payload(reference_type: FourCc, large: bool, payload: &[u8]) -> Result<Self, Error> {
        let mut reader = FieldReader::new(payload);
        let from_item_id = read_u16_or_u32(&mut reader, large)?;
        let reference_count = reader.read_u16()?;
        let to_item_ids = read_entries(
            &mut reader,
            u32::from(reference_count),
            if large { 4 } else { 2 },
            |reader| read_u16_or_u32(reader, large),
        )?;

        Ok(Self {
            reference_type,
            from_item_id,
            to_item_ids,
        })
    }
}

/// [`SingleItemTypeReferenceBox`] with the width its [`ItemReferenceBox`] gives the `item_ID`s, which writes its payload
struct SizedReference<'reference> {
    /// Box written
    reference: &'reference SingleItemTypeReferenceBox,
    /// Whether the `item_ID`s take 32 bits rather than 16
    large: bool,
}

impl BoxEncode for SizedReference<'_> {
    fn payload_len(&self) -> u64 {
        let id_len = if self.large { 4 } else { 2 };

        table_len(self.reference.to_item_ids.len().saturating_add(1), id_len).saturating_add(2)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let reference_count = u16::try_from(self.reference.to_item_ids.len())
            .map_err(|_| Error::invalid_value("reference_count"))?;
        let mut sorted = self.reference.to_item_ids.clone();
        sorted.sort_unstable();
        if has_adjacent_equal(&sorted) {
            return Err(Error::invalid_value("to_item_ID"));
        }
        write_u16_or_u32(
            writer,
            self.large,
            u64::from(self.reference.from_item_id),
            "from_item_ID",
        )?;
        writer.write_u16(reference_count)?;
        self.reference
            .to_item_ids
            .iter()
            .try_for_each(|id| write_u16_or_u32(writer, self.large, u64::from(*id), "to_item_ID"))
    }
}

/// One child of an [`ItemReferenceBox`]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub enum ItemReference {
    /// References of the type the box type gives
    Typed(SingleItemTypeReferenceBox),
    /// Child under a `uuid` box type, or one that does not decode, kept as its bytes
    Other(AnyBox),
}

/// [`ItemReferenceBox`] (`iref`), ISO/IEC 14496-12 §8.11.12
///
/// References from items to other items, one [`SingleItemTypeReferenceBox`]
/// for each item and type of reference.
///
/// The box is written in the `version` it holds: version 0 writes the
/// `item_ID`s of its references in 16 bits, version 1 in 32, and an `item_ID`
/// that does not fit version 0 is refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
/// `from_item_ID` or `to_item_ID` when the box is written.
///
/// Every child under a compact box type is read as a
/// [`SingleItemTypeReferenceBox`] of that type. A child under a `uuid` box
/// type, and one whose payload ends inside a field, is kept as its bytes. The
/// children are kept, and written back, in the order they came, which other
/// structures may index.
///
/// # Untrusted input
///
/// The box reads each child as it frames it, and allocates room for a child
/// only once it has read it, so what it allocates is bounded by the length of
/// the payload, which the caller sets.
#[doc(alias = "iref")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct ItemReferenceBox {
    /// Version of the box, 0 or 1, which sets the width of the `item_ID`s
    pub version: u8,
    /// Children, in the order they came
    pub references: Vec<ItemReference>,
}

impl BoxDefinition for ItemReferenceBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"iref");
}

impl BoxDecode for ItemReferenceBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?).version();
        if version > 1 {
            return Err(Error::unsupported_version(version));
        }
        let mut references = Vec::new();
        for child in Boxes::new(reader.take_remainder()) {
            let child = child?;
            let box_type = child.header().box_type();
            let payload = child.payload();
            let read = match box_type {
                BoxType::Compact(reference_type) => SingleItemTypeReferenceBox::decode_payload(
                    reference_type.four_cc(),
                    version == 1,
                    payload,
                )
                .map(ItemReference::Typed)
                .ok(),
                BoxType::Extended(_) => None,
            };
            references.push(read.unwrap_or_else(|| {
                ItemReference::Other(AnyBox::from_raw_bytes(box_type, payload.to_vec()))
            }));
        }

        Ok(Self {
            version,
            references,
        })
    }
}

impl BoxEncode for ItemReferenceBox {
    fn payload_len(&self) -> u64 {
        self.references
            .iter()
            .map(|child| match child {
                ItemReference::Typed(reference) => {
                    box_type_argument::encoded_len(&SizedReference {
                        reference,
                        large: self.version == 1,
                    })
                }
                ItemReference::Other(other) => other.encoded_len(),
            })
            .fold(4, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.version > 1 {
            return Err(Error::unsupported_version(self.version));
        }
        writer.write_bytes(&FullBoxHeader::new(self.version, FullBoxFlags::ZERO).to_bytes())?;
        let mut rest = writer.take_remainder();
        for child in &self.references {
            rest = match child {
                ItemReference::Typed(reference) => box_type_argument::encode(
                    reference.reference_type,
                    "referenceType",
                    &SizedReference {
                        reference,
                        large: self.version == 1,
                    },
                    rest,
                )?,
                ItemReference::Other(other) => other.encode(rest)?,
            };
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{AnyBox, BoxDecode, BoxEncode, BoxType, Error, FourCc};

    use super::{ItemReference, ItemReferenceBox, SingleItemTypeReferenceBox};
    use crate::encoded;

    /// `dimg` reference of version 0 from item 1 to the items 2 and 3
    const DIMG_V0: &[u8] = b"\0\0\0\x10dimg\0\x01\0\x02\0\x02\0\x03";

    /// `thmb` reference of version 1 from item 2^16 to item 1
    const THMB_V1: &[u8] = b"\0\0\0\x12thmb\0\x01\0\0\0\x01\0\0\0\x01";

    /// Child under a `uuid` box type
    const EXTENDED: &[u8] =
        b"\0\0\0\x1cuuid\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10\0\0\0\x07";

    fn typed(reference_type: &[u8; 4], from_item_id: u32, to_item_ids: &[u32]) -> ItemReference {
        ItemReference::Typed(SingleItemTypeReferenceBox {
            reference_type: FourCc::new(*reference_type),
            from_item_id,
            to_item_ids: to_item_ids.to_vec(),
        })
    }

    fn extended() -> ItemReference {
        ItemReference::Other(AnyBox::from_raw_bytes(
            BoxType::extended(*b"\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10"),
            b"\0\0\0\x07".to_vec(),
        ))
    }

    fn version_0() -> ItemReferenceBox {
        ItemReferenceBox {
            version: 0,
            references: vec![typed(b"dimg", 1, &[2, 3]), extended()],
        }
    }

    fn version_1() -> ItemReferenceBox {
        ItemReferenceBox {
            version: 1,
            references: vec![typed(b"thmb", 1 << 16, &[1])],
        }
    }

    #[test]
    fn reads_every_child_of_a_compact_type_as_references_of_that_type_in_either_version() {
        let version_0_bytes = [b"\0\0\0\x38iref\0\0\0\0".as_slice(), DIMG_V0, EXTENDED].concat();
        let version_1_bytes = [b"\0\0\0\x1eiref\x01\0\0\0".as_slice(), THMB_V1].concat();

        assert_eq!(
            (
                ItemReferenceBox::decode(&version_0_bytes).map(|(read, _)| read),
                ItemReferenceBox::decode(&version_1_bytes).map(|(read, _)| read)
            ),
            (Ok(version_0()), Ok(version_1()))
        );
    }

    #[test]
    fn writes_each_child_under_its_type_in_the_version_it_holds() {
        assert_eq!(
            (encoded(&version_0()), encoded(&version_1())),
            (
                [b"\0\0\0\x38iref\0\0\0\0".as_slice(), DIMG_V0, EXTENDED].concat(),
                [b"\0\0\0\x1eiref\x01\0\0\0".as_slice(), THMB_V1].concat()
            )
        );
    }

    #[test]
    fn a_child_ending_inside_its_item_ids_is_kept_as_its_bytes() {
        let expected = ItemReferenceBox {
            version: 0,
            references: vec![ItemReference::Other(AnyBox::from_raw_bytes(
                BoxType::compact(*b"cdsc"),
                b"\0\x01\0\x02\0\x02".to_vec(),
            ))],
        };

        assert_eq!(
            ItemReferenceBox::decode_payload(b"\0\0\0\0\0\0\0\x0ecdsc\0\x01\0\x02\0\x02"),
            Ok(expected)
        );
    }

    #[test]
    fn what_the_references_may_not_hold_is_refused_when_written() {
        let uuid = ItemReferenceBox {
            version: 0,
            references: vec![typed(b"uuid", 1, &[2])],
        };
        let twice = ItemReferenceBox {
            version: 0,
            references: vec![typed(b"dimg", 1, &[2, 2])],
        };
        let wide = ItemReferenceBox {
            version: 0,
            references: vec![typed(b"dimg", 1, &[1 << 16])],
        };

        assert_eq!(
            (
                uuid.encode(&mut [0; 32]),
                twice.encode(&mut [0; 32]),
                wide.encode(&mut [0; 32])
            ),
            (
                Err(Error::invalid_value("referenceType")),
                Err(Error::invalid_value("to_item_ID")),
                Err(Error::invalid_value("to_item_ID"))
            )
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused_when_read_and_when_written() {
        let mut version_2 = version_1();
        version_2.version = 2;

        assert_eq!(
            (
                ItemReferenceBox::decode_payload(b"\x02\0\0\0"),
                version_2.encode(&mut [0; 32])
            ),
            (
                Err(Error::unsupported_version(2)),
                Err(Error::unsupported_version(2))
            )
        );
    }
}
