//! [`ItemPropertyAssociationBox`], the item properties associated with each item

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFlags,
    FullBoxHeader,
};

use crate::full_box::{read_u16_or_u32, write_u16_or_u32};
use crate::table::{read_entries, table_len, write_entry_count};

/// Bit of `flags` that widens `property_index` from 7 bits to 15
const LARGE_PROPERTY_INDEX: u32 = 1;

/// Association of one item property with an item, one entry of an [`ItemPropertyAssociationEntry`]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct ItemPropertyAssociation {
    /// Whether a reader that does not support the property shall not process the item
    pub essential: bool,
    /// Index, from 1, of the property in the `ItemPropertyContainerBox`, or 0 for no property
    pub property_index: u16,
}

/// Item properties associated with one item, one entry of an [`ItemPropertyAssociationBox`]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct ItemPropertyAssociationEntry {
    /// `item_ID` of the item
    pub item_id: u32,
    /// Properties of the item, in the order they came
    ///
    /// More than 255 are refused with
    /// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
    /// `association_count` when the box is written.
    pub associations: Vec<ItemPropertyAssociation>,
}

/// [`ItemPropertyAssociationBox`] (`ipma`), ISO/IEC 14496-12 §8.11.14
///
/// The item properties associated with each item, by their index in the
/// `ItemPropertyContainerBox` of the same `ItemPropertiesBox`, each marked
/// essential or not.
///
/// The box is written in the `version` and `flags` it holds. Version 0 carries
/// `item_ID` in 16 bits, version 1 in 32; bit 0 of `flags` set widens
/// `property_index` from 7 bits to 15. A value that does not fit its width is
/// refused with [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
/// `item_ID` or `property_index` when the box is written.
///
/// # Untrusted input
///
/// Each count is held against the rest of the payload before any entry is
/// read, so what the box allocates is bounded by the length of the payload,
/// which the caller sets.
#[doc(alias = "ipma")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ItemPropertyAssociationBox {
    /// Version of the box, 0 or 1, which sets the width of `item_ID`
    pub version: u8,
    /// Flags of the box, kept as read, of which bit 0 sets the width of `property_index`
    pub flags: FullBoxFlags,
    /// Items and their properties, in the order they came, which is by increasing `item_ID`
    pub entries: Vec<ItemPropertyAssociationEntry>,
}

impl Default for ItemPropertyAssociationBox {
    fn default() -> Self {
        Self {
            version: 0,
            flags: FullBoxFlags::ZERO,
            entries: Vec::new(),
        }
    }
}

impl ItemPropertyAssociationBox {
    /// Returns whether `property_index` takes 15 bits rather than 7
    fn large_property_index(&self) -> bool {
        self.flags.bits() & LARGE_PROPERTY_INDEX != 0
    }
}

impl BoxDefinition for ItemPropertyAssociationBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"ipma");
}

impl BoxDecode for ItemPropertyAssociationBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let header = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?);
        let version = header.version();
        if version > 1 {
            return Err(Error::unsupported_version(version));
        }
        let mut read = Self {
            version,
            flags: header.flags(),
            entries: Vec::new(),
        };
        let (wide, large) = (version == 1, read.large_property_index());
        let entry_count = reader.read_u32()?;
        read.entries = read_entries(
            reader,
            entry_count,
            if wide { 4 + 1 } else { 2 + 1 },
            |reader| {
                let item_id = read_u16_or_u32(reader, wide)?;
                let [association_count] = *reader.read_bytes::<1>()?;
                let associations = read_entries(
                    reader,
                    u32::from(association_count),
                    if large { 2 } else { 1 },
                    |reader| {
                        let (essential, property_index) = if large {
                            let bits = reader.read_u16()?;
                            (bits >> 15 != 0, bits & 0x7fff)
                        } else {
                            let [bits] = *reader.read_bytes::<1>()?;
                            (bits >> 7 != 0, u16::from(bits & 0x7f))
                        };

                        Ok(ItemPropertyAssociation {
                            essential,
                            property_index,
                        })
                    },
                )?;

                Ok(ItemPropertyAssociationEntry {
                    item_id,
                    associations,
                })
            },
        )?;

        Ok(read)
    }
}

impl BoxEncode for ItemPropertyAssociationBox {
    fn payload_len(&self) -> u64 {
        let item_id_len = if self.version == 1 { 4 } else { 2 };
        let association_len = if self.large_property_index() { 2 } else { 1 };

        self.entries
            .iter()
            .map(|entry| {
                table_len(entry.associations.len(), association_len)
                    .saturating_add(item_id_len)
                    .saturating_add(1)
            })
            .fold(4 + 4, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.version > 1 {
            return Err(Error::unsupported_version(self.version));
        }
        let large = self.large_property_index();

        writer.write_bytes(&FullBoxHeader::new(self.version, self.flags).to_bytes())?;
        write_entry_count(writer, self.entries.len())?;
        for entry in &self.entries {
            write_u16_or_u32(
                writer,
                self.version == 1,
                u64::from(entry.item_id),
                "item_ID",
            )?;
            let association_count = u8::try_from(entry.associations.len())
                .map_err(|_| Error::invalid_value("association_count"))?;
            writer.write_bytes(&[association_count])?;
            for association in &entry.associations {
                if association.property_index > if large { 0x7fff } else { 0x7f } {
                    return Err(Error::invalid_value("property_index"));
                }
                let bits = u16::from(association.essential) << if large { 15 } else { 7 }
                    | association.property_index;
                if large {
                    writer.write_u16(bits)?;
                } else {
                    let [_, low] = bits.to_be_bytes();
                    writer.write_bytes(&[low])?;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error, FullBoxFlags};

    use super::{
        ItemPropertyAssociation, ItemPropertyAssociationBox, ItemPropertyAssociationEntry,
    };
    use crate::encoded;

    /// `ipma` of version 0 and 7-bit indexes: item 1 with property 1 essential and property 2, item 2 with property 3
    const IPMA_V0: &[u8] = b"\0\0\0\x19ipma\0\0\0\0\0\0\0\x02\0\x01\x02\x81\x02\0\x02\x01\x03";

    /// `ipma` of version 1 and 15-bit indexes: item 2^16 with property 300 essential
    const IPMA_V1: &[u8] = b"\0\0\0\x17ipma\x01\0\0\x01\0\0\0\x01\0\x01\0\0\x01\x81\x2c";

    fn association(essential: bool, property_index: u16) -> ItemPropertyAssociation {
        ItemPropertyAssociation {
            essential,
            property_index,
        }
    }

    fn version_0() -> ItemPropertyAssociationBox {
        ItemPropertyAssociationBox {
            version: 0,
            flags: FullBoxFlags::ZERO,
            entries: vec![
                ItemPropertyAssociationEntry {
                    item_id: 1,
                    associations: vec![association(true, 1), association(false, 2)],
                },
                ItemPropertyAssociationEntry {
                    item_id: 2,
                    associations: vec![association(false, 3)],
                },
            ],
        }
    }

    fn version_1() -> ItemPropertyAssociationBox {
        ItemPropertyAssociationBox {
            version: 1,
            flags: FullBoxFlags::new(1).unwrap(),
            entries: vec![ItemPropertyAssociationEntry {
                item_id: 1 << 16,
                associations: vec![association(true, 300)],
            }],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_in_each_width() {
        assert_eq!(
            (
                ItemPropertyAssociationBox::decode(IPMA_V0),
                ItemPropertyAssociationBox::decode(IPMA_V1)
            ),
            (
                Ok((version_0(), b"".as_slice())),
                Ok((version_1(), b"".as_slice()))
            )
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_the_widths_it_holds() {
        assert_eq!(
            (encoded(&version_0()), encoded(&version_1())),
            (IPMA_V0.to_vec(), IPMA_V1.to_vec())
        );
    }

    #[test]
    fn a_value_wider_than_its_field_is_refused_when_written() {
        let mut wide_index = version_0();
        wide_index.entries = vec![ItemPropertyAssociationEntry {
            item_id: 1,
            associations: vec![association(false, 128)],
        }];
        let mut wide_item_id = version_0();
        wide_item_id.entries = vec![ItemPropertyAssociationEntry {
            item_id: 1 << 16,
            associations: vec![],
        }];

        assert_eq!(
            (
                wide_index.encode(&mut [0; 32]),
                wide_item_id.encode(&mut [0; 32])
            ),
            (
                Err(Error::invalid_value("property_index")),
                Err(Error::invalid_value("item_ID"))
            )
        );
    }

    #[test]
    fn a_count_of_more_associations_than_the_payload_holds_is_refused() {
        assert_eq!(
            ItemPropertyAssociationBox::decode_payload(b"\0\0\0\0\0\0\0\x01\0\x01\x03\x81"),
            Err(Error::truncated_payload(14, 12))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused_when_read_and_when_written() {
        let mut version_2 = version_0();
        version_2.version = 2;

        assert_eq!(
            (
                ItemPropertyAssociationBox::decode_payload(b"\x02\0\0\0"),
                version_2.encode(&mut [0; 32])
            ),
            (
                Err(Error::unsupported_version(2)),
                Err(Error::unsupported_version(2))
            )
        );
    }
}
