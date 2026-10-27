//! [`GroupIdToNameBox`], the names of the file groups of the items

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, Utf8String,
};

use crate::full_box::{read_version_zero, write_version_zero};
use crate::infe::{read_string, write_string};
use crate::table::read_entries;

/// File group and its name, one entry of a [`GroupIdToNameBox`]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct GroupIdToNameEntry {
    /// File group, as the `group_ID`s of an `FdItemInfoExtension` name it
    pub group_id: u32,
    /// Name of the file group
    pub group_name: Utf8String,
}

/// [`GroupIdToNameBox`] (`gitn`), ISO/IEC 14496-12 §8.12.6
///
/// The names of the file groups that the item information entries of
/// version 1 name by their `group_ID`.
///
/// The Syntax of the 2026 edition reads `unsigned int(32 group_ID`; the box
/// reads it as `unsigned int(32) group_ID`, as GPAC does. More than 65535
/// entries are refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for `entry_count`
/// when the box is written.
///
/// # Untrusted input
///
/// The box holds `entry_count` against the payload before it reads an entry,
/// so what it allocates is bounded by the length of the payload, which the
/// caller sets.
#[doc(alias = "gitn")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct GroupIdToNameBox {
    /// File groups and their names, in the order they came
    pub entries: Vec<GroupIdToNameEntry>,
}

impl BoxDefinition for GroupIdToNameBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"gitn");
}

impl BoxDecode for GroupIdToNameBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;
        let entry_count = reader.read_u16()?;
        // Why not the five bytes of a `group_ID` and a terminator: the last
        // `group_name` may run to the end of the payload without one.
        let entries = read_entries(reader, u32::from(entry_count), 4, |reader| {
            Ok(GroupIdToNameEntry {
                group_id: reader.read_u32()?,
                group_name: read_string(reader)?,
            })
        })?;

        Ok(Self { entries })
    }
}

impl BoxEncode for GroupIdToNameBox {
    fn payload_len(&self) -> u64 {
        self.entries
            .iter()
            .map(|entry| entry.group_name.encoded_len().saturating_add(4))
            .fold(4 + 2, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        writer.write_u16(
            u16::try_from(self.entries.len()).map_err(|_| Error::invalid_value("entry_count"))?,
        )?;
        for entry in &self.entries {
            writer.write_u32(entry.group_id)?;
            write_string(writer, &entry.group_name)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec;

    use isobmff_core::{BoxDecode, Error, Utf8String};

    use super::{GroupIdToNameBox, GroupIdToNameEntry};
    use crate::encoded;

    /// `gitn` naming the file group 1 `news` and the file group 2 `sport`
    const GITN: &[u8] = b"\0\0\0\x21gitn\0\0\0\0\0\x02\0\0\0\x01news\0\0\0\0\x02sport\0";

    fn entry(group_id: u32, group_name: &str) -> GroupIdToNameEntry {
        GroupIdToNameEntry {
            group_id,
            group_name: Utf8String::new(String::from(group_name)).unwrap(),
        }
    }

    fn group_names() -> GroupIdToNameBox {
        GroupIdToNameBox {
            entries: vec![entry(1, "news"), entry(2, "sport")],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            GroupIdToNameBox::decode(GITN),
            Ok((group_names(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&group_names()), GITN);
    }

    #[test]
    fn a_last_name_without_its_terminator_reads_to_the_end_and_is_written_with_one() {
        let read =
            GroupIdToNameBox::decode_payload(b"\0\0\0\0\0\x02\0\0\0\x01news\0\0\0\0\x02sport")
                .unwrap();

        assert_eq!(encoded(&read), GITN);
    }

    #[test]
    fn a_count_of_more_entries_than_the_payload_holds_is_refused() {
        assert_eq!(
            GroupIdToNameBox::decode_payload(b"\0\0\0\0\0\x02\0\0\0\x01a\0"),
            Err(Error::truncated_payload(14, 12))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            GroupIdToNameBox::decode_payload(b"\x01\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
