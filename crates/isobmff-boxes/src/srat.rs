//! [`SamplingRateBox`], the sampling rate of an audio stream

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::full_box::{read_version_zero, write_version_zero};

/// [`SamplingRateBox`] (`srat`), ISO/IEC 14496-12 §12.2.3
///
/// Actual sampling rate of the audio, for a rate the `samplerate` of the
/// sample entry cannot carry. Either an [`AudioSampleEntry`] or an
/// [`AudioSampleEntryV1`] may hold it; in an [`AudioSampleEntryV1`] it
/// overrides `samplerate`.
///
/// The box reads version 0 alone. The `flags` are read past and written as
/// zero.
///
/// [`AudioSampleEntry`]: crate::AudioSampleEntry
/// [`AudioSampleEntryV1`]: crate::AudioSampleEntryV1
#[doc(alias = "srat")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct SamplingRateBox {
    /// Sampling rate of the audio, in samples per second
    pub sampling_rate: u32,
}

impl BoxDefinition for SamplingRateBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"srat");
}

impl BoxDecode for SamplingRateBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {
            sampling_rate: reader.read_u32()?,
        })
    }
}

impl BoxEncode for SamplingRateBox {
    fn payload_len(&self) -> u64 {
        4 + 4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        writer.write_u32(self.sampling_rate)
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, Error};

    use super::SamplingRateBox;
    use crate::encoded;

    /// `srat` of 96000 samples per second
    const SRAT: &[u8] = b"\0\0\0\x10srat\0\0\0\0\0\x01\x77\0";

    fn sampling_rate() -> SamplingRateBox {
        SamplingRateBox {
            sampling_rate: 96_000,
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            SamplingRateBox::decode(SRAT),
            Ok((sampling_rate(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&sampling_rate()), SRAT);
    }

    #[test]
    fn a_payload_ending_inside_a_field_is_refused() {
        assert_eq!(
            SamplingRateBox::decode_payload(b"\0\0\0\0\0\x01"),
            Err(Error::truncated_payload(8, 6))
        );
    }

    #[test]
    fn a_version_other_than_0_is_refused() {
        assert_eq!(
            SamplingRateBox::decode_payload(b"\x01\0\0\0\0\x01\x77\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
