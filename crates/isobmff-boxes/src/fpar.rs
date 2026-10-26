//! [`FilePartitionBox`], the partitioning of a source file into source blocks and symbols

use alloc::vec::Vec;

use isobmff_core::{
    Base64String, BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter,
    FullBoxFlags, FullBoxHeader,
};

use crate::full_box::{read_u16_or_u32, write_u16_or_u32};
use crate::sample_entry::read_string;
use crate::table::{TableEntry, entries_len, read_entries, write_entries};

/// Run of consecutive source blocks of one size, one entry of a [`FilePartitionBox`]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct FilePartitionEntry {
    /// Number of consecutive source blocks of `block_size`
    pub block_count: u16,
    /// Size of a block, in bytes
    pub block_size: u32,
}

impl TableEntry for FilePartitionEntry {
    const ENCODED_LEN: u64 = 2 + 4;

    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            block_count: reader.read_u16()?,
            block_size: reader.read_u32()?,
        })
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u16(self.block_count)?;
        writer.write_u32(self.block_size)
    }
}

/// [`FilePartitionBox`] (`fpar`), ISO/IEC 14496-12 §8.12.3
///
/// The source file an item holds, and how it is partitioned into source
/// blocks and symbols and FEC encoded for delivery over ALC/LCT or FLUTE.
///
/// The box is written in the `version` it holds: version 0 carries `item_ID`
/// and `entry_count` in 16 bits, version 1 in 32. A value that does not fit
/// version 0 is refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for the field
/// when the box is written.
///
/// # Untrusted input
///
/// The box holds `entry_count` against the payload before it reads an entry,
/// so what it allocates is bounded by the length of the payload, which the
/// caller sets.
#[doc(alias = "fpar")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct FilePartitionBox {
    /// Version of the box, 0 or 1, which sets the width of `item_ID` and `entry_count`
    pub version: u8,
    /// Item in the `ItemLocationBox` the partitioning applies to
    pub item_id: u32,
    /// Target ALC/LCT or FLUTE packet payload size of the partitioning algorithm
    pub packet_payload_size: u16,
    /// FEC encoding scheme, a FEC Encoding ID of IETF RFC 5052
    pub fec_encoding_id: u8,
    /// FEC encoder of an Under-Specified FEC scheme, scoped by `fec_encoding_id`
    pub fec_instance_id: u16,
    /// Largest number of source symbols per source block
    pub max_source_block_length: u16,
    /// Size of one encoding symbol, in bytes
    pub encoding_symbol_length: u16,
    /// Largest number of encoding symbols a source block may be given, for the FEC schemes it is relevant to
    pub max_number_of_encoding_symbols: u16,
    /// FEC-OTI-Scheme-Specific-Info, whose definition depends on `fec_encoding_id`
    pub scheme_specific_info: Base64String,
    /// Runs of source blocks that partition the file from its beginning, in order
    pub entries: Vec<FilePartitionEntry>,
}

impl BoxDefinition for FilePartitionBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"fpar");
}

impl BoxDecode for FilePartitionBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?).version();
        if version > 1 {
            return Err(Error::unsupported_version(version));
        }
        let wide = version == 1;
        let item_id = read_u16_or_u32(reader, wide)?;
        let packet_payload_size = reader.read_u16()?;
        reader.read_bytes::<1>()?;
        let [fec_encoding_id] = *reader.read_bytes::<1>()?;
        let fec_instance_id = reader.read_u16()?;
        let max_source_block_length = reader.read_u16()?;
        let encoding_symbol_length = reader.read_u16()?;
        let max_number_of_encoding_symbols = reader.read_u16()?;
        let scheme_specific_info = Base64String::from_slice(read_string(reader)?);
        let entry_count = read_u16_or_u32(reader, wide)?;

        Ok(Self {
            version,
            item_id,
            packet_payload_size,
            fec_encoding_id,
            fec_instance_id,
            max_source_block_length,
            encoding_symbol_length,
            max_number_of_encoding_symbols,
            scheme_specific_info,
            entries: read_entries(
                reader,
                entry_count,
                FilePartitionEntry::ENCODED_LEN,
                FilePartitionEntry::read,
            )?,
        })
    }
}

impl BoxEncode for FilePartitionBox {
    fn payload_len(&self) -> u64 {
        let wide_len = if self.version == 1 { 4 } else { 2 };
        [
            4 + 2 + 1 + 1 + 2 + 2 + 2 + 2,
            wide_len,
            wide_len,
            self.scheme_specific_info.encoded_len(),
            entries_len(&self.entries),
        ]
        .into_iter()
        .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.version > 1 {
            return Err(Error::unsupported_version(self.version));
        }
        let wide = self.version == 1;
        writer.write_bytes(&FullBoxHeader::new(self.version, FullBoxFlags::ZERO).to_bytes())?;
        write_u16_or_u32(writer, wide, u64::from(self.item_id), "item_ID")?;
        writer.write_u16(self.packet_payload_size)?;
        writer.write_bytes(&[0, self.fec_encoding_id])?;
        writer.write_u16(self.fec_instance_id)?;
        writer.write_u16(self.max_source_block_length)?;
        writer.write_u16(self.encoding_symbol_length)?;
        writer.write_u16(self.max_number_of_encoding_symbols)?;
        let rest = self.scheme_specific_info.encode(writer.take_remainder())?;
        let mut writer = FieldWriter::new(rest);
        write_u16_or_u32(
            &mut writer,
            wide,
            u64::try_from(self.entries.len()).unwrap_or(u64::MAX),
            "entry_count",
        )?;
        write_entries(&mut writer, &self.entries)?;
        writer.finish()
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec;

    use isobmff_core::{Base64String, BoxDecode, BoxEncode, Error};

    use super::{FilePartitionBox, FilePartitionEntry};
    use crate::encoded;

    /// `fpar` of version 0: item 1, 1400-byte payloads, FEC encoding 0, `AAEC` as its scheme-specific info and two blocks of 8192 bytes
    const FPAR_VERSION_0: &[u8] = b"\0\0\0\x27fpar\0\0\0\0\0\x01\x05\x78\0\0\0\0\0\x40\x05\x78\0\x40AAEC\0\0\x01\0\x02\0\0\x20\0";

    /// The same box in version 1, of 32-bit `item_ID` and `entry_count`
    const FPAR_VERSION_1: &[u8] = b"\0\0\0\x2bfpar\x01\0\0\0\0\0\0\x01\x05\x78\0\0\0\0\0\x40\x05\x78\0\x40AAEC\0\0\0\0\x01\0\x02\0\0\x20\0";

    fn file_partition(version: u8) -> FilePartitionBox {
        FilePartitionBox {
            version,
            item_id: 1,
            packet_payload_size: 1400,
            fec_encoding_id: 0,
            fec_instance_id: 0,
            max_source_block_length: 64,
            encoding_symbol_length: 1400,
            max_number_of_encoding_symbols: 64,
            scheme_specific_info: Base64String::new(String::from("AAEC")).unwrap(),
            entries: vec![FilePartitionEntry {
                block_count: 2,
                block_size: 8192,
            }],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_in_either_version() {
        assert_eq!(
            FilePartitionBox::decode(FPAR_VERSION_0),
            Ok((file_partition(0), b"".as_slice()))
        );
        assert_eq!(
            FilePartitionBox::decode(FPAR_VERSION_1),
            Ok((file_partition(1), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_either_version() {
        assert_eq!(encoded(&file_partition(0)), FPAR_VERSION_0);
        assert_eq!(encoded(&file_partition(1)), FPAR_VERSION_1);
    }

    #[test]
    fn the_reserved_byte_is_read_on_past_and_written_as_zero() {
        let (read, _) = FilePartitionBox::decode(b"\0\0\0\x27fpar\0\0\0\0\0\x01\x05\x78\xff\0\0\0\0\x40\x05\x78\0\x40AAEC\0\0\x01\0\x02\0\0\x20\0").unwrap();

        assert_eq!(encoded(&read), FPAR_VERSION_0);
    }

    #[test]
    fn an_item_id_wider_than_version_0_carries_is_refused_when_written() {
        let partition = FilePartitionBox {
            item_id: 0x1_0000,
            ..file_partition(0)
        };

        assert_eq!(
            partition.encode(&mut [0; 64]).map(|_| ()),
            Err(Error::invalid_value("item_ID"))
        );
    }

    #[test]
    fn scheme_specific_info_that_is_not_base64_is_refused_when_written() {
        let (read, _) = FilePartitionBox::decode(b"\0\0\0\x27fpar\0\0\0\0\0\x01\x05\x78\0\0\0\0\0\x40\x05\x78\0\x40AA*C\0\0\x01\0\x02\0\0\x20\0").unwrap();

        assert_eq!(
            read.encode(&mut [0; 64]).map(|_| ()),
            Err(Error::invalid_base64_string())
        );
    }

    #[test]
    fn a_count_of_more_entries_than_the_payload_holds_is_refused() {
        assert_eq!(
            FilePartitionBox::decode_payload(
                b"\0\0\0\0\0\x01\x05\x78\0\0\0\0\0\x40\x05\x78\0\x40AAEC\0\0\x02\0\0\x20\0"
            ),
            Err(Error::truncated_payload(37, 29))
        );
    }

    #[test]
    fn a_payload_ending_inside_the_fields_is_refused() {
        assert_eq!(
            FilePartitionBox::decode_payload(b"\0\0\0\0\0"),
            Err(Error::truncated_payload(6, 5))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            FilePartitionBox::decode_payload(b"\x02\0\0\0"),
            Err(Error::unsupported_version(2))
        );
    }
}
