//! [`AudioRenderingIndicationBox`], the reproduction channel layout a preselection prefers

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::full_box::{read_version_zero, write_version_zero};

/// [`AudioRenderingIndicationBox`] (`ardi`), ISO/IEC 14496-12 §12.2.8
///
/// Hint for a preferred reproduction channel layout. It lies in a
/// [`PreselectionGroupBox`](crate::PreselectionGroupBox).
///
/// The box reads version 0 alone. The `flags` are read past and written as
/// zero.
#[doc(alias = "ardi")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct AudioRenderingIndicationBox {
    /// Preferred reproduction channel layout, as Table 12 codes it
    ///
    /// * 0: no preference given
    /// * 1: stereo
    /// * 2: two-dimensional, such as 5.1 multi-channel
    /// * 3: three-dimensional
    /// * 4: pre-rendered for consumption with headphones
    /// * 5 to 255: reserved
    pub audio_rendering_indication: u8,
}

impl BoxDefinition for AudioRenderingIndicationBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"ardi");
}

impl BoxDecode for AudioRenderingIndicationBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;
        let [audio_rendering_indication] = *reader.read_bytes::<1>()?;

        Ok(Self {
            audio_rendering_indication,
        })
    }
}

impl BoxEncode for AudioRenderingIndicationBox {
    fn payload_len(&self) -> u64 {
        4 + 1
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        writer.write_bytes(&[self.audio_rendering_indication])
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, Error};

    use super::AudioRenderingIndicationBox;
    use crate::encoded;

    /// `ardi` preferring a three-dimensional layout
    const ARDI: &[u8] = b"\0\0\0\x0dardi\0\0\0\0\x03";

    fn audio_rendering_indication() -> AudioRenderingIndicationBox {
        AudioRenderingIndicationBox {
            audio_rendering_indication: 3,
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            AudioRenderingIndicationBox::decode(ARDI),
            Ok((audio_rendering_indication(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&audio_rendering_indication()), ARDI);
    }

    #[test]
    fn a_payload_ending_inside_a_field_is_refused() {
        assert_eq!(
            AudioRenderingIndicationBox::decode_payload(b"\0\0\0\0"),
            Err(Error::truncated_payload(5, 4))
        );
    }

    #[test]
    fn a_version_other_than_0_is_refused() {
        assert_eq!(
            AudioRenderingIndicationBox::decode_payload(b"\x01\0\0\0\x03"),
            Err(Error::unsupported_version(1))
        );
    }
}
