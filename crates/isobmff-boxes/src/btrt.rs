//! [`BitRateBox`] (`btrt`), ISO/IEC 14496-12 §8.5.2

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

/// Box a sample entry may hold to state the bit rate of its stream
///
/// [`BitRateBox`] (`btrt`), ISO/IEC 14496-12 §8.5.2. ISO/IEC 14496-15 names the
/// same box `MPEG4BitRateBox`.
#[doc(alias = "btrt")]
#[doc(alias = "MPEG4BitRateBox")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct BitRateBox {
    buffer_size_db: u32,
    max_bitrate: u32,
    avg_bitrate: u32,
}

impl BitRateBox {
    /// Creates the box from the decoding buffer size in bytes and the maximum
    /// and average bit rates in bits per second
    #[must_use]
    pub const fn new(buffer_size_db: u32, max_bitrate: u32, avg_bitrate: u32) -> Self {
        Self {
            buffer_size_db,
            max_bitrate,
            avg_bitrate,
        }
    }

    /// Returns the size of the decoding buffer for the stream, in bytes
    #[must_use]
    pub const fn buffer_size_db(&self) -> u32 {
        self.buffer_size_db
    }

    /// Returns the maximum rate in bits per second over any one-second window
    #[must_use]
    pub const fn max_bitrate(&self) -> u32 {
        self.max_bitrate
    }

    /// Returns the average rate in bits per second over the whole presentation
    #[must_use]
    pub const fn avg_bitrate(&self) -> u32 {
        self.avg_bitrate
    }
}

impl BoxDefinition for BitRateBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"btrt");
}

impl BoxDecode for BitRateBox {
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends before the fields do.
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
        12
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(self.buffer_size_db)?;
        writer.write_u32(self.max_bitrate)?;
        writer.write_u32(self.avg_bitrate)
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::BitRateBox;

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let bit_rate = BitRateBox::new(0x1000, 5_000_000, 3_000_000);
        let mut payload = [0; 12];

        bit_rate.encode_payload(&mut payload).unwrap();

        assert_eq!(payload, *b"\0\0\x10\0\0\x4c\x4b\x40\0\x2d\xc6\xc0");
        assert_eq!(BitRateBox::decode_payload(&payload).unwrap(), bit_rate);
    }

    #[test]
    fn a_payload_ending_before_the_fields_is_rejected_as_truncated() {
        assert_eq!(
            BitRateBox::decode_payload(&[0; 11]),
            Err(Error::truncated_payload(12, 11))
        );
    }
}
