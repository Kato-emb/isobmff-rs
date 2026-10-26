//! [`ItemLocationBox`] and the [`ItemLocation`]s and [`ItemLocationExtent`]s it holds

use alloc::vec;
use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFlags,
    FullBoxHeader,
};

use crate::full_box::{read_u16_or_u32, write_u16_or_u32};
use crate::table::{read_entries, table_len};

/// Extent of an item, a contiguous run of the bytes of the resource it is formed from, one entry of an [`ItemLocation`]
///
/// A field whose size the [`ItemLocationBox`] sets to 0 is not in the box, and
/// holds 0.
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct ItemLocationExtent {
    /// Index, from 1, of the `iloc` item reference of the item that the extent lies in, for the construction method 2
    ///
    /// It is 0 where `index_size` is 0, which implies the index 1.
    pub item_reference_index: u64,
    /// Offset in bytes of the extent from the data origin of the construction method
    pub extent_offset: u64,
    /// Length in bytes of the extent, where 0 stands for the rest of the data from `extent_offset`
    pub extent_length: u64,
}

/// Location of one item, one entry of an [`ItemLocationBox`]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct ItemLocation {
    /// `item_ID` of the item
    pub item_id: u32,
    /// Where the extents of the item lie: 0 in a file, 1 in the `ItemDataBox`, 2 in other items
    ///
    /// It is 0 in a box of version 0, which carries no such field.
    pub construction_method: u8,
    /// 0 for the file that holds the box, or the index, from 1, of a data reference in the `DataInformationBox` of the `MetaBox`
    pub data_reference_index: u16,
    /// Value added to the offsets within the data the item references, 0 where `base_offset_size` is 0
    pub base_offset: u64,
    /// Extents of the item, which concatenated in this order form it
    pub extents: Vec<ItemLocationExtent>,
}

/// [`ItemLocationBox`] (`iloc`), ISO/IEC 14496-12 §8.11.3
///
/// A directory of the resources of the items of a `MetaBox`: the container
/// each lies in, its offset in that container and its length.
///
/// The box is written in the `version` it holds, 0, 1 or 2. Version 0 has no
/// `construction_method` and no `item_reference_index`, and version 2 widens
/// `item_ID` and the item count from 16 bits to 32. `offset_size`,
/// `length_size`, `base_offset_size` and `index_size` set the length in bytes
/// of the fields of the same names. Each is 0, 4 or 8. A box of any other
/// length up to 8 reads, as GPAC reads it, and one of a length above 8 does
/// not; either is refused when it is written. `index_size` is 0 in version 0,
/// and a `construction_method` other than 0 or an `index_size` other than 0 is
/// refused there when the box is written.
///
/// What the box refuses is refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for the field
/// concerned. So is, when the box is written, a value that does not fit its
/// field, and a `construction_method` other than 0, 1 or 2. An item of more than one extent whose extents take no byte is
/// refused with [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
/// `extent_count` when it is read and when it is written: §8.11.3.2.5 lets a
/// field be implied only for an item of one extent.
///
/// The box holds the offsets as they were read. An offset of the
/// construction method 0 is a position in a file, which the writer of the
/// file adjusts when the data moves.
///
/// # Untrusted input
///
/// Each count is held against the rest of the payload before any entry is
/// read, so what the box allocates is bounded by the length of the payload,
/// which the caller sets.
#[doc(alias = "iloc")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct ItemLocationBox {
    /// Version of the box, 0, 1 or 2
    pub version: u8,
    /// Length in bytes of `extent_offset`
    pub offset_size: u8,
    /// Length in bytes of `extent_length`
    pub length_size: u8,
    /// Length in bytes of `base_offset`
    pub base_offset_size: u8,
    /// Length in bytes of `item_reference_index`, 0 in version 0
    pub index_size: u8,
    /// Locations of the items, in the order they came
    pub items: Vec<ItemLocation>,
}

impl ItemLocationBox {
    /// Returns each of `offset_size`, `length_size`, `base_offset_size` and `index_size` with its name
    fn sizes(&self) -> [(u8, &'static str); 4] {
        [
            (self.offset_size, "offset_size"),
            (self.length_size, "length_size"),
            (self.base_offset_size, "base_offset_size"),
            (self.index_size, "index_size"),
        ]
    }

    /// Returns the bytes each extent takes under the sizes the box holds
    fn extent_len(&self) -> u64 {
        [self.index_size, self.offset_size, self.length_size]
            .into_iter()
            .map(u64::from)
            .fold(0, u64::saturating_add)
    }

    /// Returns the bytes an item of no extent takes under the version and sizes the box holds
    fn item_len(&self) -> u64 {
        let fields_len = match self.version {
            0 => 2 + 2 + 2,
            1 => 2 + 2 + 2 + 2,
            _ => 4 + 2 + 2 + 2,
        };

        u64::from(self.base_offset_size).saturating_add(fields_len)
    }
}

/// Reads a field of `size` bytes, 0 where `size` is 0
///
/// `size` is at most 8.
///
/// # Errors
///
/// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
///   payload ends inside the field.
fn read_sized(reader: &mut FieldReader<'_>, size: u8) -> Result<u64, Error> {
    let bytes = reader.read_slice(usize::from(size))?;

    Ok(bytes
        .iter()
        .fold(0, |value, byte| value << 8 | u64::from(*byte)))
}

/// Writes `value` as a field of `size` bytes, which writes nothing where `size` is 0
///
/// `size` is 0, 4 or 8.
///
/// # Errors
///
/// * [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) naming `field`:
///   `value` does not fit `size` bytes.
fn write_sized(
    writer: &mut FieldWriter<'_>,
    size: u8,
    value: u64,
    field: &'static str,
) -> Result<(), Error> {
    let bytes = value.to_be_bytes();
    let (high, low) = bytes.split_at(bytes.len().saturating_sub(usize::from(size)));
    if high.iter().any(|byte| *byte != 0) {
        return Err(Error::invalid_value(field));
    }
    writer.write_slice(low)
}

impl BoxDefinition for ItemLocationBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"iloc");
}

impl BoxDecode for ItemLocationBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?).version();
        if version > 2 {
            return Err(Error::unsupported_version(version));
        }
        let [offset_and_length, base_offset_and_index] = *reader.read_bytes::<2>()?;
        let mut read = Self {
            version,
            offset_size: offset_and_length >> 4,
            length_size: offset_and_length & 0x0f,
            base_offset_size: base_offset_and_index >> 4,
            index_size: if version == 0 {
                0
            } else {
                base_offset_and_index & 0x0f
            },
            items: Vec::new(),
        };
        if let Some((_, field)) = read.sizes().iter().find(|(size, _)| *size > 8) {
            return Err(Error::invalid_value(field));
        }
        let wide = version == 2;
        let extent_len = read.extent_len();
        let item_count = read_u16_or_u32(reader, wide)?;
        read.items = read_entries(reader, item_count, read.item_len(), |reader| {
            let item_id = read_u16_or_u32(reader, wide)?;
            let construction_method = if version == 0 {
                0
            } else {
                let [_, method] = *reader.read_bytes::<2>()?;
                method & 0x0f
            };
            let data_reference_index = reader.read_u16()?;
            let base_offset = read_sized(reader, read.base_offset_size)?;
            let extent_count = reader.read_u16()?;
            let extents = if extent_len == 0 {
                // Why not `read_entries`: an extent that takes no byte bounds no
                // count by the payload, so only the one extent §8.11.3.2.5 lets
                // imply its fields is read.
                match extent_count {
                    0 | 1 => vec![ItemLocationExtent::default(); usize::from(extent_count)],
                    _ => return Err(Error::invalid_value("extent_count")),
                }
            } else {
                read_entries(reader, u32::from(extent_count), extent_len, |reader| {
                    Ok(ItemLocationExtent {
                        item_reference_index: read_sized(reader, read.index_size)?,
                        extent_offset: read_sized(reader, read.offset_size)?,
                        extent_length: read_sized(reader, read.length_size)?,
                    })
                })?
            };

            Ok(ItemLocation {
                item_id,
                construction_method,
                data_reference_index,
                base_offset,
                extents,
            })
        })?;

        Ok(read)
    }
}

impl BoxEncode for ItemLocationBox {
    fn payload_len(&self) -> u64 {
        let fields_len = if self.version == 2 {
            4 + 2 + 4
        } else {
            4 + 2 + 2
        };

        let (extent_len, item_len) = (self.extent_len(), self.item_len());

        self.items
            .iter()
            .map(|item| table_len(item.extents.len(), extent_len).saturating_add(item_len))
            .fold(fields_len, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.version > 2 {
            return Err(Error::unsupported_version(self.version));
        }
        if let Some((_, field)) = self
            .sizes()
            .iter()
            .find(|(size, _)| !matches!(size, 0 | 4 | 8))
        {
            return Err(Error::invalid_value(field));
        }
        if self.version == 0 && self.index_size != 0 {
            return Err(Error::invalid_value("index_size"));
        }
        let wide = self.version == 2;
        let extent_len = self.extent_len();

        writer.write_bytes(&FullBoxHeader::new(self.version, FullBoxFlags::ZERO).to_bytes())?;
        writer.write_bytes(&[
            self.offset_size << 4 | self.length_size,
            self.base_offset_size << 4 | self.index_size,
        ])?;
        write_u16_or_u32(
            writer,
            wide,
            u64::try_from(self.items.len()).unwrap_or(u64::MAX),
            "item_count",
        )?;
        for item in &self.items {
            write_u16_or_u32(writer, wide, u64::from(item.item_id), "item_ID")?;
            match (self.version, item.construction_method) {
                (0, 0) => {}
                (1 | 2, method @ 0..=2) => writer.write_bytes(&[0, method])?,
                _ => return Err(Error::invalid_value("construction_method")),
            }
            writer.write_u16(item.data_reference_index)?;
            write_sized(
                writer,
                self.base_offset_size,
                item.base_offset,
                "base_offset",
            )?;
            let extent_count = u16::try_from(item.extents.len())
                .map_err(|_| Error::invalid_value("extent_count"))?;
            if extent_len == 0 && extent_count > 1 {
                return Err(Error::invalid_value("extent_count"));
            }
            writer.write_u16(extent_count)?;
            for extent in &item.extents {
                write_sized(
                    writer,
                    self.index_size,
                    extent.item_reference_index,
                    "item_reference_index",
                )?;
                write_sized(
                    writer,
                    self.offset_size,
                    extent.extent_offset,
                    "extent_offset",
                )?;
                write_sized(
                    writer,
                    self.length_size,
                    extent.extent_length,
                    "extent_length",
                )?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::{ItemLocation, ItemLocationBox, ItemLocationExtent};
    use crate::encoded;

    /// `iloc` of version 0, 4-byte offsets and lengths, no base offset: item 1 in two extents of the file, at 0x100 for 0x10 bytes and at 0x200 for 0x20
    const ILOC_V0: &[u8] = b"\0\0\0\x26iloc\0\0\0\0\x44\0\0\x01\
        \0\x01\0\0\0\x02\0\0\x01\0\0\0\0\x10\0\0\x02\0\0\0\0\x20";

    /// `iloc` of version 1, 8-byte offsets, no lengths, 4-byte base offsets and indexes: item 2 in the `ItemDataBox`, base offset 4, at 8; item 3 in item reference 1, at 0
    const ILOC_V1: &[u8] = b"\0\0\0\x40iloc\x01\0\0\0\x80\x44\0\x02\
        \0\x02\0\x01\0\0\0\0\0\x04\0\x01\0\0\0\0\0\0\0\0\0\0\0\x08\
        \0\x03\0\x02\0\0\0\0\0\0\0\x01\0\0\0\x01\0\0\0\0\0\0\0\0";

    /// `iloc` of version 2, no offset, length, base offset or index: item 2^16 in the one extent its fields imply
    const ILOC_V2: &[u8] = b"\0\0\0\x1ciloc\x02\0\0\0\0\0\0\0\0\x01\0\x01\0\0\0\0\0\0\0\x01";

    fn item(
        item_id: u32,
        construction_method: u8,
        base_offset: u64,
        extents: &[(u64, u64, u64)],
    ) -> ItemLocation {
        ItemLocation {
            item_id,
            construction_method,
            data_reference_index: 0,
            base_offset,
            extents: extents
                .iter()
                .map(
                    |&(item_reference_index, extent_offset, extent_length)| ItemLocationExtent {
                        item_reference_index,
                        extent_offset,
                        extent_length,
                    },
                )
                .collect(),
        }
    }

    fn version_0() -> ItemLocationBox {
        ItemLocationBox {
            version: 0,
            offset_size: 4,
            length_size: 4,
            items: vec![item(1, 0, 0, &[(0, 0x100, 0x10), (0, 0x200, 0x20)])],
            ..ItemLocationBox::default()
        }
    }

    fn version_1() -> ItemLocationBox {
        ItemLocationBox {
            version: 1,
            offset_size: 8,
            base_offset_size: 4,
            index_size: 4,
            items: vec![item(2, 1, 4, &[(0, 8, 0)]), item(3, 2, 0, &[(1, 0, 0)])],
            ..ItemLocationBox::default()
        }
    }

    fn version_2() -> ItemLocationBox {
        ItemLocationBox {
            version: 2,
            items: vec![item(1 << 16, 0, 0, &[(0, 0, 0)])],
            ..ItemLocationBox::default()
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_in_each_version() {
        assert_eq!(
            (
                ItemLocationBox::decode(ILOC_V0),
                ItemLocationBox::decode(ILOC_V1),
                ItemLocationBox::decode(ILOC_V2)
            ),
            (
                Ok((version_0(), b"".as_slice())),
                Ok((version_1(), b"".as_slice())),
                Ok((version_2(), b"".as_slice()))
            )
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_the_version_it_holds() {
        assert_eq!(
            (
                encoded(&version_0()),
                encoded(&version_1()),
                encoded(&version_2())
            ),
            (ILOC_V0.to_vec(), ILOC_V1.to_vec(), ILOC_V2.to_vec())
        );
    }

    #[test]
    fn a_field_size_up_to_8_bytes_outside_those_allowed_reads_but_is_refused_when_written() {
        let (read, _) = ItemLocationBox::decode(
            b"\0\0\0\x19iloc\0\0\0\0\x12\0\0\x01\0\x01\0\0\0\x01\xaa\xbb\xcc",
        )
        .unwrap();

        assert_eq!(
            (read.clone(), read.encode(&mut [0; 25])),
            (
                ItemLocationBox {
                    offset_size: 1,
                    length_size: 2,
                    items: vec![item(1, 0, 0, &[(0, 0xaa, 0xbbcc)])],
                    ..ItemLocationBox::default()
                },
                Err(Error::invalid_value("offset_size"))
            )
        );
    }

    #[test]
    fn a_field_size_beyond_8_bytes_is_refused() {
        assert_eq!(
            ItemLocationBox::decode_payload(b"\0\0\0\0\x90\0\0\x01\0\x01\0\0\0\x01"),
            Err(Error::invalid_value("offset_size"))
        );
    }

    #[test]
    fn more_than_one_extent_taking_no_byte_is_refused_when_read_and_when_written() {
        let mut two_implied = version_2();
        two_implied.items = vec![item(1, 0, 0, &[(0, 0, 0), (0, 0, 0)])];

        assert_eq!(
            (
                ItemLocationBox::decode_payload(
                    b"\x02\0\0\0\0\0\0\0\0\x01\0\0\0\x01\0\0\0\0\0\x02"
                ),
                two_implied.encode(&mut [0; 32])
            ),
            (
                Err(Error::invalid_value("extent_count")),
                Err(Error::invalid_value("extent_count"))
            )
        );
    }

    #[test]
    fn a_value_wider_than_its_field_is_refused_when_written() {
        let mut wide_offset = version_0();
        wide_offset.items = vec![item(1, 0, 0, &[(0, 1 << 32, 0)])];
        let mut wide_item_id = version_1();
        wide_item_id.items = vec![item(1 << 16, 0, 0, &[])];

        assert_eq!(
            (
                wide_offset.encode(&mut [0; 64]),
                wide_item_id.encode(&mut [0; 64])
            ),
            (
                Err(Error::invalid_value("extent_offset")),
                Err(Error::invalid_value("item_ID"))
            )
        );
    }

    #[test]
    fn what_version_0_does_not_carry_is_refused_when_written() {
        let mut with_method = version_0();
        with_method.items = vec![item(1, 1, 0, &[])];
        let mut with_index = version_0();
        with_index.index_size = 4;

        assert_eq!(
            (
                with_method.encode(&mut [0; 64]),
                with_index.encode(&mut [0; 64])
            ),
            (
                Err(Error::invalid_value("construction_method")),
                Err(Error::invalid_value("index_size"))
            )
        );
    }

    #[test]
    fn a_construction_method_beyond_those_defined_is_refused_when_written() {
        let mut unknown = version_1();
        unknown.items = vec![item(1, 3, 0, &[])];

        assert_eq!(
            unknown.encode(&mut [0; 64]),
            Err(Error::invalid_value("construction_method"))
        );
    }

    #[test]
    fn a_payload_ending_inside_an_extent_is_refused() {
        assert_eq!(
            ItemLocationBox::decode_payload(
                b"\0\0\0\0\x44\0\0\x01\0\x01\0\0\0\x02\0\0\x01\0\0\0\0\x10\0\0\x02\0\0\0"
            ),
            Err(Error::truncated_payload(30, 28))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused_when_read_and_when_written() {
        let mut version_3 = version_0();
        version_3.version = 3;

        assert_eq!(
            (
                ItemLocationBox::decode_payload(b"\x03\0\0\0"),
                version_3.encode(&mut [0; 64])
            ),
            (
                Err(Error::unsupported_version(3)),
                Err(Error::unsupported_version(3))
            )
        );
    }
}
