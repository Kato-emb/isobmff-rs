//! [`BitRateBox`], the bit rates and decoding buffer size of a stream

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

/// [`BitRateBox`] (`btrt`), ISO/IEC 14496-12 §8.5.2
///
/// Bit rate information of the stream a sample entry describes, which can
/// configure the decoding buffer. Any sample entry may hold one.
#[doc(alias = "btrt", alias = "MPEG4BitRateBox")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct BitRateBox {
    /// Size of the decoding buffer of the stream, in bytes
    pub buffer_size_db: u32,
    /// Largest rate over any window of one second, in bits per second
    pub max_bitrate: u32,
    /// Average rate over the whole presentation, in bits per second
    pub avg_bitrate: u32,
}

impl BoxDefinition for BitRateBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"btrt");
}

impl BoxDecode for BitRateBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            buffer_size_db: reader.read_u32()?,
            max_bitrate: reader.read_u32()?,
            avg_bitrate: reader.read_u32()?,
        })
    }
}

impl BoxEncode for BitRateBox {
    fn payload_len(&self) -> u64 {
        3 * 4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(self.buffer_size_db)?;
        writer.write_u32(self.max_bitrate)?;
        writer.write_u32(self.avg_bitrate)
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, Error};

    use super::BitRateBox;
    use crate::encoded;

    /// `btrt` of a buffer of 4096 bytes, at most 2 Mbit/s and 1 Mbit/s on average
    const BTRT: &[u8] = b"\0\0\0\x14btrt\0\0\x10\0\0\x1e\x84\x80\0\x0f\x42\x40";

    fn bit_rate() -> BitRateBox {
        BitRateBox {
            buffer_size_db: 4096,
            max_bitrate: 2_000_000,
            avg_bitrate: 1_000_000,
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(BitRateBox::decode(BTRT), Ok((bit_rate(), b"".as_slice())));
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&bit_rate()), BTRT);
    }

    #[test]
    fn a_payload_ending_inside_a_field_is_refused() {
        assert_eq!(
            BitRateBox::decode_payload(b"\0\0\x10\0\0\x1e\x84\x80\0\x0f"),
            Err(Error::truncated_payload(12, 10))
        );
    }
}
