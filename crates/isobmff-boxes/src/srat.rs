//! [`SamplingRateBox`] (`srat`), ISO/IEC 14496-12 §12.2.3

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFields,
    FullBoxFlags,
};

/// Box a version 1 audio sample entry holds to state the actual sampling rate
///
/// [`SamplingRateBox`] (`srat`), ISO/IEC 14496-12 §12.2.3. The `samplerate`
/// field of the entry is a 16.16 number whose integer part cannot reach past
/// `u16`; this box states the rate whole, as a 32-bit integer.
#[doc(alias = "srat")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SamplingRateBox {
    sampling_rate: u32,
}

impl SamplingRateBox {
    /// Creates the box from the sampling rate of the audio, in samples a second
    #[must_use]
    pub const fn new(sampling_rate: u32) -> Self {
        Self { sampling_rate }
    }

    /// Returns the actual sampling rate of the audio, in samples a second
    #[must_use]
    pub const fn sampling_rate(&self) -> u32 {
        self.sampling_rate
    }
}

impl BoxDefinition for SamplingRateBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"srat");
}

impl BoxDecode for SamplingRateBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends before the fields do.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version != 0 {
            return Err(Error::unsupported_version(version));
        }

        Ok(Self {
            sampling_rate: reader.read_u32()?,
        })
    }
}

impl BoxEncode for SamplingRateBox {
    fn payload_len(&self) -> u64 {
        8
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxFields::new(0, FullBoxFlags::ZERO).to_bytes())?;
        writer.write_u32(self.sampling_rate)
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::SamplingRateBox;

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let sampling_rate = SamplingRateBox::new(96_000);
        let mut payload = [0; 8];

        sampling_rate.encode_payload(&mut payload).unwrap();

        assert_eq!(payload, *b"\0\0\0\0\0\x01\x77\0");
        assert_eq!(
            SamplingRateBox::decode_payload(&payload).unwrap(),
            sampling_rate
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        assert_eq!(
            SamplingRateBox::decode_payload(b"\x01\0\0\0\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
