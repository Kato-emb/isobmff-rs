//! [`AmbientViewingEnvironmentBox`], the ambient viewing environment a video was mastered for

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

/// [`AmbientViewingEnvironmentBox`] (`amve`), ISO/IEC 14496-12 §12.1.9
///
/// Characteristics of the nominal ambient viewing environment for the display
/// of the video, as the ambient viewing environment SEI message of ITU-T H.265
/// | ISO/IEC 23008-2 gives them.
#[doc(alias = "amve")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct AmbientViewingEnvironmentBox {
    /// Illuminance of the environment, in units of 0.0001 lux
    pub ambient_illuminance: u32,
    /// Normalized x chromaticity coordinate of the ambient light, in units of 0.00002
    pub ambient_light_x: u16,
    /// Normalized y chromaticity coordinate of the ambient light, in units of 0.00002
    pub ambient_light_y: u16,
}

impl BoxDefinition for AmbientViewingEnvironmentBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"amve");
}

impl BoxDecode for AmbientViewingEnvironmentBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            ambient_illuminance: reader.read_u32()?,
            ambient_light_x: reader.read_u16()?,
            ambient_light_y: reader.read_u16()?,
        })
    }
}

impl BoxEncode for AmbientViewingEnvironmentBox {
    fn payload_len(&self) -> u64 {
        4 + 2 + 2
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(self.ambient_illuminance)?;
        writer.write_u16(self.ambient_light_x)?;
        writer.write_u16(self.ambient_light_y)
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, Error};

    use super::AmbientViewingEnvironmentBox;
    use crate::encoded;

    /// `amve` of 314 lux under the white point of D65
    const AMVE: &[u8] = b"\0\0\0\x10amve\0\x2f\xe9\xa0\x3d\x13\x40\x42";

    fn ambient_viewing_environment() -> AmbientViewingEnvironmentBox {
        AmbientViewingEnvironmentBox {
            ambient_illuminance: 3_140_000,
            ambient_light_x: 15635,
            ambient_light_y: 16450,
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            AmbientViewingEnvironmentBox::decode(AMVE),
            Ok((ambient_viewing_environment(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&ambient_viewing_environment()), AMVE);
    }

    #[test]
    fn a_payload_ending_inside_a_field_is_refused() {
        assert_eq!(
            AmbientViewingEnvironmentBox::decode_payload(b"\0\x2f\xe9\xa0\x3d\x13\x40"),
            Err(Error::truncated_payload(8, 7))
        );
    }
}
