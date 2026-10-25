//! [`CompactSampleSizeBox`]

use alloc::vec::Vec;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::full_box::{read_version_zero, write_version_zero};
use crate::table::write_entry_count;

/// [`CompactSampleSizeBox`] (`stz2`), ISO/IEC 14496-12 §8.7.3.3
///
/// The size of each sample of the track, in a table of entries 4, 8 or 16 bits
/// wide. Entries of 4 bits pack two to a byte, the earlier sample in the high
/// half, and the last byte of an odd number of them is padded with zeros.
///
/// A `field_size` other than 4, 8 or 16 is refused, read or written, with
/// [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue). An
/// `entry_size` wider than `field_size` is refused when the box is written,
/// with [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue).
///
/// [`Default`] is the box of no sample, of 16-bit entries.
#[doc(alias = "stz2")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct CompactSampleSizeBox {
    /// Width of every entry of the table, in bits: 4, 8 or 16
    pub field_size: u8,
    /// `entry_size` of every sample, in bytes, indexed by its number
    pub entry_sizes: Vec<u16>,
}

impl Default for CompactSampleSizeBox {
    fn default() -> Self {
        Self {
            field_size: 16,
            entry_sizes: Vec::new(),
        }
    }
}

/// Returns how many bytes `sample_count` entries of `field_size` bits take, saturating at `u64::MAX`
fn table_len(sample_count: u64, field_size: u8) -> u64 {
    sample_count
        .saturating_mul(u64::from(field_size))
        .div_ceil(8)
}

impl BoxDefinition for CompactSampleSizeBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"stz2");
}

impl BoxDecode for CompactSampleSizeBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;
        let [_, _, _, field_size] = *reader.read_bytes::<4>()?;
        if !matches!(field_size, 4 | 8 | 16) {
            return Err(Error::unsupported_value("field_size"));
        }
        let sample_count = reader.read_u32()?;
        let table_len = table_len(u64::from(sample_count), field_size);
        // Why not leave the bound to read_slice: on a target whose usize is
        // narrower than the table length, the conversion below saturates, and
        // the failure would name that length rather than the table's.
        reader.require(table_len)?;
        let table = reader.read_slice(usize::try_from(table_len).unwrap_or(usize::MAX))?;
        let sample_count = usize::try_from(sample_count).unwrap_or(usize::MAX);
        let entry_sizes = match field_size {
            4 => table
                .iter()
                .flat_map(|byte| [u16::from(byte >> 4), u16::from(byte & 0x0f)])
                .take(sample_count)
                .collect(),
            8 => table.iter().copied().map(u16::from).collect(),
            _ => table
                .as_chunks::<2>()
                .0
                .iter()
                .copied()
                .map(u16::from_be_bytes)
                .collect(),
        };

        Ok(Self {
            field_size,
            entry_sizes,
        })
    }
}

impl BoxEncode for CompactSampleSizeBox {
    fn payload_len(&self) -> u64 {
        let sample_count = u64::try_from(self.entry_sizes.len()).unwrap_or(u64::MAX);

        table_len(sample_count, self.field_size).saturating_add(4 + 4 + 4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let maximum = match self.field_size {
            4 => 0x0f,
            8 => 0xff,
            16 => 0xffff,
            _ => return Err(Error::unsupported_value("field_size")),
        };
        if self
            .entry_sizes
            .iter()
            .any(|entry_size| *entry_size > maximum)
        {
            return Err(Error::invalid_value("entry_size"));
        }

        write_version_zero(writer)?;
        writer.write_bytes(&[0, 0, 0, self.field_size])?;
        write_entry_count(writer, self.entry_sizes.len())?;
        match self.field_size {
            4 => {
                for pair in self.entry_sizes.chunks(2) {
                    let high = pair.first().copied().unwrap_or_default();
                    let low = pair.get(1).copied().unwrap_or_default();
                    let [_, byte] = (high << 4 | low).to_be_bytes();
                    writer.write_bytes(&[byte])?;
                }
            }
            8 => {
                for entry_size in &self.entry_sizes {
                    let [_, byte] = entry_size.to_be_bytes();
                    writer.write_bytes(&[byte])?;
                }
            }
            _ => {
                for entry_size in &self.entry_sizes {
                    writer.write_u16(*entry_size)?;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::CompactSampleSizeBox;
    use crate::encoded;

    /// `stz2` of three 4-bit entries, of 1, 15 and 7 bytes, the last byte padded
    const STZ2_4: &[u8] = b"\0\0\0\x16stz2\0\0\0\0\0\0\0\x04\0\0\0\x03\x1f\x70";

    /// `stz2` of three 8-bit entries, of 1, 255 and 7 bytes
    const STZ2_8: &[u8] = b"\0\0\0\x17stz2\0\0\0\0\0\0\0\x08\0\0\0\x03\x01\xff\x07";

    /// `stz2` of two 16-bit entries, of 1 and 0x1234 bytes
    const STZ2_16: &[u8] = b"\0\0\0\x18stz2\0\0\0\0\0\0\0\x10\0\0\0\x02\0\x01\x12\x34";

    fn compact_sample_size(field_size: u8, entry_sizes: &[u16]) -> CompactSampleSizeBox {
        CompactSampleSizeBox {
            field_size,
            entry_sizes: entry_sizes.to_vec(),
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_in_every_field_size() {
        assert_eq!(
            [
                CompactSampleSizeBox::decode(STZ2_4),
                CompactSampleSizeBox::decode(STZ2_8),
                CompactSampleSizeBox::decode(STZ2_16)
            ],
            [
                Ok((compact_sample_size(4, &[1, 15, 7]), b"".as_slice())),
                Ok((compact_sample_size(8, &[1, 255, 7]), b"".as_slice())),
                Ok((compact_sample_size(16, &[1, 0x1234]), b"".as_slice()))
            ]
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_every_field_size() {
        assert_eq!(
            [
                encoded(&compact_sample_size(4, &[1, 15, 7])),
                encoded(&compact_sample_size(8, &[1, 255, 7])),
                encoded(&compact_sample_size(16, &[1, 0x1234]))
            ],
            [STZ2_4.to_vec(), STZ2_8.to_vec(), STZ2_16.to_vec()]
        );
    }

    #[test]
    fn an_even_number_of_4_bit_entries_fills_its_last_byte() {
        assert_eq!(
            CompactSampleSizeBox::decode_payload(b"\0\0\0\0\0\0\0\x04\0\0\0\x02\x1f"),
            Ok(compact_sample_size(4, &[1, 15]))
        );
    }

    #[test]
    fn a_field_size_other_than_4_8_or_16_is_refused_read_or_written() {
        assert_eq!(
            (
                CompactSampleSizeBox::decode_payload(b"\0\0\0\0\0\0\0\x20\0\0\0\0"),
                compact_sample_size(32, &[]).encode(&mut [0; 20])
            ),
            (
                Err(Error::unsupported_value("field_size")),
                Err(Error::unsupported_value("field_size"))
            )
        );
    }

    #[test]
    fn an_entry_wider_than_the_field_size_is_refused_when_written() {
        assert_eq!(
            compact_sample_size(4, &[16]).encode(&mut [0; 21]),
            Err(Error::invalid_value("entry_size"))
        );
    }

    #[test]
    fn a_sample_count_of_more_entries_than_the_payload_holds_is_refused() {
        assert_eq!(
            CompactSampleSizeBox::decode_payload(b"\0\0\0\0\0\0\0\x10\0\0\0\x02\0\x01"),
            Err(Error::truncated_payload(16, 14))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            CompactSampleSizeBox::decode_payload(b"\x01\0\0\0\0\0\0\x10\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
