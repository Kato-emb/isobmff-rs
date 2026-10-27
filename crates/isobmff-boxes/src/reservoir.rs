//! [`FecReservoirBox`] and [`FileReservoirBox`], the items that hold the reservoirs of a source file, block by block

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFlags,
    FullBoxHeader,
};

use crate::full_box::{read_u16_or_u32, write_u16_or_u32};
use crate::table::{read_entries, table_len};

/// Reservoir of one source block: the item that holds it and how many symbols it holds, one entry of a [`FecReservoirBox`] or a [`FileReservoirBox`]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct ReservoirEntry {
    /// Item that holds the reservoir
    pub item_id: u32,
    /// Number of symbols the reservoir holds
    pub symbol_count: u32,
}

/// Reads the version and flags, `entry_count` and the entries it counts, of `item_ID` 32 bits wide in version 1 and 16 in version 0
///
/// # Errors
///
/// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
///   payload ends inside the fields or holds fewer entries than it counts.
/// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the
///   version is neither 0 nor 1.
fn read_reservoirs(reader: &mut FieldReader<'_>) -> Result<(u8, Vec<ReservoirEntry>), Error> {
    let version = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?).version();
    if version > 1 {
        return Err(Error::unsupported_version(version));
    }
    let wide = version == 1;
    let entry_count = read_u16_or_u32(reader, wide)?;
    let entries = read_entries(reader, entry_count, entry_len(version), |reader| {
        Ok(ReservoirEntry {
            item_id: read_u16_or_u32(reader, wide)?,
            symbol_count: reader.read_u32()?,
        })
    })?;

    Ok((version, entries))
}

/// Returns the bytes an entry takes in `version`
fn entry_len(version: u8) -> u64 {
    if version == 1 { 4 + 4 } else { 2 + 4 }
}

/// Returns the bytes the payload of a box of `version` and `entries` takes
fn reservoirs_len(version: u8, entries: &[ReservoirEntry]) -> u64 {
    let fields_len = if version == 1 { 4 + 4 } else { 4 + 2 };

    table_len(entries.len(), entry_len(version)).saturating_add(fields_len)
}

/// Writes the version, the flags of zero, `entry_count` and the entries, as [`read_reservoirs`] reads them
///
/// # Errors
///
/// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion):
///   `version` is neither 0 nor 1.
/// * [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
///   `entry_count` or `item_ID`: the value does not fit version 0.
fn write_reservoirs(
    writer: &mut FieldWriter<'_>,
    version: u8,
    entries: &[ReservoirEntry],
) -> Result<(), Error> {
    if version > 1 {
        return Err(Error::unsupported_version(version));
    }
    let wide = version == 1;
    writer.write_bytes(&FullBoxHeader::new(version, FullBoxFlags::ZERO).to_bytes())?;
    write_u16_or_u32(
        writer,
        wide,
        u64::try_from(entries.len()).unwrap_or(u64::MAX),
        "entry_count",
    )?;
    entries.iter().try_for_each(|entry| {
        write_u16_or_u32(writer, wide, u64::from(entry.item_id), "item_ID")?;
        writer.write_u32(entry.symbol_count)
    })
}

/// [`FecReservoirBox`] (`fecr`), ISO/IEC 14496-12 §8.12.4
///
/// The FEC reservoirs of the source file a [`FilePartitionBox`](crate::FilePartitionBox)
/// identifies, each stored as an item, starting with that of the first source
/// block and going on through the source blocks in order.
///
/// The box is written in the `version` it holds: version 0 carries
/// `entry_count` and `item_ID` in 16 bits, version 1 in 32. A value that does
/// not fit version 0 is refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for the field
/// when the box is written.
///
/// # Untrusted input
///
/// The box holds `entry_count` against the payload before it reads an entry,
/// so what it allocates is bounded by the length of the payload, which the
/// caller sets.
#[doc(alias = "fecr", alias = "FECReservoirBox")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct FecReservoirBox {
    /// Version of the box, 0 or 1, which sets the width of `entry_count` and `item_ID`
    pub version: u8,
    /// FEC reservoir of each source block, of the repair symbols it holds
    pub entries: Vec<ReservoirEntry>,
}

impl BoxDefinition for FecReservoirBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"fecr");
}

impl BoxDecode for FecReservoirBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let (version, entries) = read_reservoirs(reader)?;

        Ok(Self { version, entries })
    }
}

impl BoxEncode for FecReservoirBox {
    fn payload_len(&self) -> u64 {
        reservoirs_len(self.version, &self.entries)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_reservoirs(writer, self.version, &self.entries)
    }
}

/// [`FileReservoirBox`] (`fire`), ISO/IEC 14496-12 §8.12.7
///
/// The File reservoirs of the source file a [`FilePartitionBox`](crate::FilePartitionBox)
/// identifies, each stored as an item, starting with that of the first source
/// block and going on through the source blocks in order.
///
/// The box is written in the `version` it holds: version 0 carries
/// `entry_count` and `item_ID` in 16 bits, version 1 in 32. A value that does
/// not fit version 0 is refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for the field
/// when the box is written.
///
/// # Untrusted input
///
/// The box holds `entry_count` against the payload before it reads an entry,
/// so what it allocates is bounded by the length of the payload, which the
/// caller sets.
#[doc(alias = "fire")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct FileReservoirBox {
    /// Version of the box, 0 or 1, which sets the width of `entry_count` and `item_ID`
    pub version: u8,
    /// File reservoir of each source block, of the source symbols it holds
    pub entries: Vec<ReservoirEntry>,
}

impl BoxDefinition for FileReservoirBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"fire");
}

impl BoxDecode for FileReservoirBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let (version, entries) = read_reservoirs(reader)?;

        Ok(Self { version, entries })
    }
}

impl BoxEncode for FileReservoirBox {
    fn payload_len(&self) -> u64 {
        reservoirs_len(self.version, &self.entries)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_reservoirs(writer, self.version, &self.entries)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::{FecReservoirBox, FileReservoirBox, ReservoirEntry};
    use crate::encoded;

    /// `fecr` of version 0: items 2 and 3, of 4 and 5 repair symbols
    const FECR_VERSION_0: &[u8] = b"\0\0\0\x1afecr\0\0\0\0\0\x02\0\x02\0\0\0\x04\0\x03\0\0\0\x05";

    /// The same box in version 1, of 32-bit `entry_count` and `item_ID`
    const FECR_VERSION_1: &[u8] =
        b"\0\0\0\x20fecr\x01\0\0\0\0\0\0\x02\0\0\0\x02\0\0\0\x04\0\0\0\x03\0\0\0\x05";

    fn entries() -> Vec<ReservoirEntry> {
        vec![
            ReservoirEntry {
                item_id: 2,
                symbol_count: 4,
            },
            ReservoirEntry {
                item_id: 3,
                symbol_count: 5,
            },
        ]
    }

    #[test]
    fn a_fec_reservoir_box_reads_from_the_bytes_its_syntax_lays_out_in_either_version() {
        for (version, bytes) in [(0, FECR_VERSION_0), (1, FECR_VERSION_1)] {
            assert_eq!(
                FecReservoirBox::decode(bytes),
                Ok((
                    FecReservoirBox {
                        version,
                        entries: entries()
                    },
                    b"".as_slice()
                ))
            );
        }
    }

    #[test]
    fn a_fec_reservoir_box_writes_the_bytes_its_syntax_lays_out_in_either_version() {
        for (version, bytes) in [(0, FECR_VERSION_0), (1, FECR_VERSION_1)] {
            assert_eq!(
                encoded(&FecReservoirBox {
                    version,
                    entries: entries()
                }),
                bytes
            );
        }
    }

    #[test]
    fn a_file_reservoir_box_reads_and_writes_the_bytes_its_syntax_lays_out() {
        let bytes: &[u8] = b"\0\0\0\x1afire\0\0\0\0\0\x02\0\x02\0\0\0\x04\0\x03\0\0\0\x05";
        let reservoirs = FileReservoirBox {
            version: 0,
            entries: entries(),
        };

        assert_eq!(
            FileReservoirBox::decode(bytes),
            Ok((reservoirs.clone(), b"".as_slice()))
        );
        assert_eq!(encoded(&reservoirs), bytes);
    }

    #[test]
    fn an_item_id_wider_than_version_0_carries_is_refused_when_written() {
        let reservoirs = FileReservoirBox {
            version: 0,
            entries: vec![ReservoirEntry {
                item_id: 0x1_0000,
                symbol_count: 1,
            }],
        };

        assert_eq!(
            reservoirs.encode(&mut [0; 32]).map(|_| ()),
            Err(Error::invalid_value("item_ID"))
        );
    }

    #[test]
    fn a_count_of_more_entries_than_the_payload_holds_is_refused() {
        assert_eq!(
            FecReservoirBox::decode_payload(b"\0\0\0\0\0\x02\0\x02\0\0\0\x04"),
            Err(Error::truncated_payload(18, 12))
        );
    }

    #[test]
    fn a_payload_ending_inside_the_count_is_refused() {
        assert_eq!(
            FileReservoirBox::decode_payload(b"\x01\0\0\0\0\0"),
            Err(Error::truncated_payload(8, 6))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            FecReservoirBox::decode_payload(b"\x02\0\0\0"),
            Err(Error::unsupported_version(2))
        );
        assert_eq!(
            FileReservoirBox::decode_payload(b"\x02\0\0\0"),
            Err(Error::unsupported_version(2))
        );
    }
}
