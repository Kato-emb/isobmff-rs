//! [`ItemInfoBox`], the information about the items of a meta box

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFlags,
    FullBoxHeader,
};

use crate::full_box::{read_u16_or_u32, write_u16_or_u32};
use crate::infe::ItemInfoEntry;

/// [`ItemInfoBox`] (`iinf`), ISO/IEC 14496-12 §8.11.6
///
/// The information about the items of a `MetaBox`, one [`ItemInfoEntry`] for
/// each, sorted by increasing `item_ID`.
///
/// The box is written in the `version` it holds: version 0 counts the entries
/// in 16 bits, version 1 in 32. More entries than version 0 counts are
/// refused with [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
/// `entry_count` when the box is written. An entry that does not read, such
/// as one of a version it does not read, fails the box.
///
/// # Untrusted input
///
/// The box holds the count against the payload before it reads an entry, and
/// allocates room for an entry only once it has read it, so what it allocates
/// is bounded by the length of the payload, which the caller sets.
#[doc(alias = "iinf")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct ItemInfoBox {
    /// Version of the box, 0 or 1, which sets the width of `entry_count`
    pub version: u8,
    /// Information about each item, in the order they came
    pub item_infos: Vec<ItemInfoEntry>,
}

impl BoxDefinition for ItemInfoBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"iinf");
}

impl BoxDecode for ItemInfoBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?).version();
        if version > 1 {
            return Err(Error::unsupported_version(version));
        }
        let entry_count = read_u16_or_u32(reader, version == 1)?;
        // Why not the twelve bytes of a full box: the count is held against
        // the header of a box alone, which every entry takes at least.
        reader.require(u64::from(entry_count).saturating_mul(8))?;
        let mut rest = reader.take_remainder();
        // Why not room for `entry_count` entries up front: an entry may take
        // few more bytes than its header, while an `ItemInfoEntry` takes
        // several times that, so the room would outgrow the payload.
        let mut item_infos = Vec::new();
        for _ in 0..entry_count {
            let (entry, after) = ItemInfoEntry::decode(rest)?;
            item_infos.push(entry);
            rest = after;
        }

        Ok(Self {
            version,
            item_infos,
        })
    }
}

impl BoxEncode for ItemInfoBox {
    fn payload_len(&self) -> u64 {
        self.item_infos.iter().map(BoxEncode::encoded_len).fold(
            if self.version == 1 { 4 + 4 } else { 4 + 2 },
            u64::saturating_add,
        )
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.version > 1 {
            return Err(Error::unsupported_version(self.version));
        }
        writer.write_bytes(&FullBoxHeader::new(self.version, FullBoxFlags::ZERO).to_bytes())?;
        write_u16_or_u32(
            writer,
            self.version == 1,
            u64::try_from(self.item_infos.len()).unwrap_or(u64::MAX),
            "entry_count",
        )?;
        let mut rest = writer.take_remainder();
        for entry in &self.item_infos {
            rest = entry.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error, FourCc, Utf8String};

    use super::ItemInfoBox;
    use crate::encoded;
    use crate::infe::{ItemInfoEntry, ItemType};

    /// `infe` of version 2 and `item_type` `hvc1`: item 1, of no name
    const INFE_1: &[u8] = b"\0\0\0\x15infe\x02\0\0\0\0\x01\0\0hvc1\0";

    /// `infe` of version 2 and `item_type` `Exif`: item 2, named `e`
    const INFE_2: &[u8] = b"\0\0\0\x16infe\x02\0\0\0\0\x02\0\0Exife\0";

    fn entry(item_id: u32, item_name: &str, item_type: &[u8; 4]) -> ItemInfoEntry {
        ItemInfoEntry {
            item_id,
            item_name: Utf8String::new(String::from(item_name)).unwrap(),
            item_type: ItemType::Other {
                item_type: FourCc::new(*item_type),
                data: vec![],
            },
            ..ItemInfoEntry::default()
        }
    }

    fn item_info(version: u8) -> ItemInfoBox {
        ItemInfoBox {
            version,
            item_infos: vec![entry(1, "", b"hvc1"), entry(2, "e", b"Exif")],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_in_either_version() {
        let version_0 = [b"\0\0\0\x39iinf\0\0\0\0\0\x02".as_slice(), INFE_1, INFE_2].concat();
        let version_1 = [
            b"\0\0\0\x3biinf\x01\0\0\0\0\0\0\x02".as_slice(),
            INFE_1,
            INFE_2,
        ]
        .concat();

        assert_eq!(
            (
                ItemInfoBox::decode(&version_0).map(|(read, _)| read),
                ItemInfoBox::decode(&version_1).map(|(read, _)| read)
            ),
            (Ok(item_info(0)), Ok(item_info(1)))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_the_version_it_holds() {
        assert_eq!(
            (encoded(&item_info(0)), encoded(&item_info(1))),
            (
                [b"\0\0\0\x39iinf\0\0\0\0\0\x02".as_slice(), INFE_1, INFE_2].concat(),
                [
                    b"\0\0\0\x3biinf\x01\0\0\0\0\0\0\x02".as_slice(),
                    INFE_1,
                    INFE_2
                ]
                .concat()
            )
        );
    }

    #[test]
    fn a_count_of_more_entries_than_the_payload_holds_is_refused() {
        assert_eq!(
            ItemInfoBox::decode_payload(&[b"\0\0\0\0\0\x03".as_slice(), INFE_1, INFE_2].concat()),
            Err(Error::truncated_header(8, 0))
        );
    }

    #[test]
    fn a_payload_ending_inside_the_count_is_refused() {
        assert_eq!(
            ItemInfoBox::decode_payload(b"\x01\0\0\0\0\0"),
            Err(Error::truncated_payload(8, 6))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused_when_read_and_when_written() {
        assert_eq!(
            (
                ItemInfoBox::decode_payload(b"\x02\0\0\0"),
                item_info(2).encode(&mut [0; 64])
            ),
            (
                Err(Error::unsupported_version(2)),
                Err(Error::unsupported_version(2))
            )
        );
    }
}
