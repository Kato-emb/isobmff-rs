//! [`CompositionOffsetBox`] and the [`CompositionOffsetEntry`]s it holds

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFlags,
    FullBoxHeader,
};

use crate::full_box::{read_composition_offset, write_composition_offset};
use crate::table::{read_entries, table_len, write_entry_count};

/// Run of consecutive samples of one composition offset, one entry of a [`CompositionOffsetBox`]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct CompositionOffsetEntry {
    /// Number of consecutive samples that have the offset `sample_offset`
    pub sample_count: u32,
    /// Offset of the composition timestamp of each of these samples from its
    /// decoding timestamp, in the timescale of the media
    ///
    /// Version 0 of the box carries it unsigned in 32 bits, version 1 signed
    /// in 32 bits.
    pub sample_offset: i64,
}

/// [`CompositionOffsetBox`] (`ctts`), ISO/IEC 14496-12 §8.6.1.3
///
/// The offsets from the decoding timestamp of each sample to its composition
/// timestamp, as runs of samples of one offset. A track whose composition and
/// decoding timestamps are the same for every sample has no such box.
///
/// The box is written in the `version` it holds. A `sample_offset` outside
/// what that version carries is refused when the box is written, with
/// [`OutOfRange`](isobmff_core::ErrorKind::OutOfRange).
#[doc(alias = "ctts")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct CompositionOffsetBox {
    /// Version of the box: 0 for unsigned offsets, 1 for signed ones
    pub version: u8,
    /// Runs of samples, in decoding order
    pub entries: Vec<CompositionOffsetEntry>,
}

impl BoxDefinition for CompositionOffsetBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"ctts");
}

impl BoxDecode for CompositionOffsetBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?).version();
        if version > 1 {
            return Err(Error::unsupported_version(version));
        }
        let entry_count = reader.read_u32()?;
        let entries = read_entries(reader, entry_count, 8, |reader| {
            Ok(CompositionOffsetEntry {
                sample_count: reader.read_u32()?,
                sample_offset: read_composition_offset(reader, version)?,
            })
        })?;

        Ok(Self { version, entries })
    }
}

impl BoxEncode for CompositionOffsetBox {
    fn payload_len(&self) -> u64 {
        table_len(self.entries.len(), 8).saturating_add(4 + 4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.version > 1 {
            return Err(Error::unsupported_version(self.version));
        }

        writer.write_bytes(&FullBoxHeader::new(self.version, FullBoxFlags::ZERO).to_bytes())?;
        write_entry_count(writer, self.entries.len())?;
        for entry in &self.entries {
            writer.write_u32(entry.sample_count)?;
            write_composition_offset(writer, self.version, entry.sample_offset)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error, FieldWidth};

    use super::{CompositionOffsetBox, CompositionOffsetEntry};
    use crate::encoded;

    /// `ctts` of version 0: one sample offset by 10, then two by `0xFFFF_FFF6`
    const CTTS_UNSIGNED: &[u8] =
        b"\0\0\0\x20ctts\0\0\0\0\0\0\0\x02\0\0\0\x01\0\0\0\x0a\0\0\0\x02\xff\xff\xff\xf6";

    /// `ctts` of version 1: one sample offset by 10, then two by -10
    const CTTS_SIGNED: &[u8] =
        b"\0\0\0\x20ctts\x01\0\0\0\0\0\0\x02\0\0\0\x01\0\0\0\x0a\0\0\0\x02\xff\xff\xff\xf6";

    fn composition_offset(version: u8, second_offset: i64) -> CompositionOffsetBox {
        CompositionOffsetBox {
            version,
            entries: vec![
                CompositionOffsetEntry {
                    sample_count: 1,
                    sample_offset: 10,
                },
                CompositionOffsetEntry {
                    sample_count: 2,
                    sample_offset: second_offset,
                },
            ],
        }
    }

    #[test]
    fn version_0_reads_its_offsets_unsigned() {
        assert_eq!(
            CompositionOffsetBox::decode(CTTS_UNSIGNED),
            Ok((composition_offset(0, 0xffff_fff6), b"".as_slice()))
        );
    }

    #[test]
    fn version_1_reads_its_offsets_signed() {
        assert_eq!(
            CompositionOffsetBox::decode(CTTS_SIGNED),
            Ok((composition_offset(1, -10), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_either_version() {
        assert_eq!(
            (
                encoded(&composition_offset(0, 0xffff_fff6)),
                encoded(&composition_offset(1, -10))
            ),
            (CTTS_UNSIGNED.to_vec(), CTTS_SIGNED.to_vec())
        );
    }

    #[test]
    fn an_offset_its_version_does_not_carry_is_refused_when_written() {
        assert_eq!(
            (
                composition_offset(0, -10).encode(&mut [0; 32]),
                composition_offset(1, 0xffff_fff6).encode(&mut [0; 32])
            ),
            (
                Err(Error::out_of_range(
                    (-10_i64).cast_unsigned(),
                    FieldWidth::Compact
                )),
                Err(Error::out_of_range(0xffff_fff6, FieldWidth::Compact))
            )
        );
    }

    #[test]
    fn a_payload_ending_inside_an_entry_is_refused() {
        assert_eq!(
            CompositionOffsetBox::decode_payload(b"\0\0\0\0\0\0\0\x01\0\0\0\x01\0\0"),
            Err(Error::truncated_payload(16, 14))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            CompositionOffsetBox::decode_payload(b"\x02\0\0\0\0\0\0\0"),
            Err(Error::unsupported_version(2))
        );
    }
}
