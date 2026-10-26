//! [`PaddingBitsBox`]

use alloc::vec::Vec;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::full_box::{read_version_zero, write_version_zero};
use crate::table::write_entry_count;

/// [`PaddingBitsBox`] (`padb`), ISO/IEC 14496-12 §8.7.6
///
/// The number of bits each sample is padded with at its end, to fill its last
/// byte. Two samples share a byte, the earlier one in its high half (`pad1`),
/// and the second half of the last byte of an odd number of samples is
/// unused.
///
/// A value above 7 is refused when the box is written, with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for `pad1` or
/// `pad2`, the half it would be written in.
#[doc(alias = "padb")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct PaddingBitsBox {
    /// Number of padding bits, from 0 to 7, at the end of every sample, in sample order
    pub pads: Vec<u8>,
}

impl BoxDefinition for PaddingBitsBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"padb");
}

impl BoxDecode for PaddingBitsBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;
        let sample_count = reader.read_u32()?;
        let table_len = u64::from(sample_count).div_ceil(2);
        let table = reader.read_slice(usize::try_from(table_len).unwrap_or(usize::MAX))?;

        Ok(Self {
            pads: table
                .iter()
                .flat_map(|byte| [byte >> 4 & 0b111, byte & 0b111])
                .take(usize::try_from(sample_count).unwrap_or(usize::MAX))
                .collect(),
        })
    }
}

impl BoxEncode for PaddingBitsBox {
    fn payload_len(&self) -> u64 {
        u64::try_from(self.pads.len())
            .unwrap_or(u64::MAX)
            .div_ceil(2)
            .saturating_add(4 + 4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if let Some(index) = self.pads.iter().position(|pad| *pad > 0b111) {
            let field = if index.is_multiple_of(2) {
                "pad1"
            } else {
                "pad2"
            };
            return Err(Error::invalid_value(field));
        }

        write_version_zero(writer)?;
        write_entry_count(writer, self.pads.len())?;
        self.pads.chunks(2).try_for_each(|pair| {
            let pad1 = pair.first().copied().unwrap_or_default();
            let pad2 = pair.get(1).copied().unwrap_or_default();
            writer.write_bytes(&[pad1 << 4 | pad2])
        })
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::PaddingBitsBox;
    use crate::encoded;

    /// `padb` of three samples, padded with 1, 7 and 3 bits
    const PADB: &[u8] = b"\0\0\0\x12padb\0\0\0\0\0\0\0\x03\x17\x30";

    fn padding_bits() -> PaddingBitsBox {
        PaddingBitsBox {
            pads: vec![1, 7, 3],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            PaddingBitsBox::decode(PADB),
            Ok((padding_bits(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&padding_bits()), PADB);
    }

    #[test]
    fn reserved_bits_are_read_on_past_and_written_as_zero() {
        let read = PaddingBitsBox::decode_payload(b"\0\0\0\0\0\0\0\x02\x9f").unwrap();

        assert_eq!(
            (encoded(&read), read),
            (
                b"\0\0\0\x11padb\0\0\0\0\0\0\0\x02\x17".to_vec(),
                PaddingBitsBox { pads: vec![1, 7] }
            )
        );
    }

    #[test]
    fn a_pad_above_7_is_refused_when_written() {
        assert_eq!(
            PaddingBitsBox {
                pads: vec![1, 8, 3]
            }
            .encode(&mut [0; 18]),
            Err(Error::invalid_value("pad2"))
        );
    }

    #[test]
    fn a_sample_count_of_more_pads_than_the_payload_holds_is_refused() {
        assert_eq!(
            PaddingBitsBox::decode_payload(b"\0\0\0\0\0\0\0\x03\x17"),
            Err(Error::truncated_payload(10, 9))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            PaddingBitsBox::decode_payload(b"\x01\0\0\0\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
