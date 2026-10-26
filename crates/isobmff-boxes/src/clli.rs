//! [`ContentLightLevelBox`], the light level of the content of a video

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

/// [`ContentLightLevelBox`] (`clli`), ISO/IEC 14496-12 §12.1.6
///
/// Light level of the content, as the content light level information SEI
/// message of ITU-T H.265 | ISO/IEC 23008-2 gives it. A value of 0 may code an
/// unknown one, as CTA-861-G provides.
#[doc(alias = "clli")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct ContentLightLevelBox {
    /// Upper bound of the light level of any pixel, in candelas per square metre
    pub max_content_light_level: u16,
    /// Upper bound of the average light level of a picture, in candelas per square metre
    pub max_pic_average_light_level: u16,
}

impl BoxDefinition for ContentLightLevelBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"clli");
}

impl BoxDecode for ContentLightLevelBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            max_content_light_level: reader.read_u16()?,
            max_pic_average_light_level: reader.read_u16()?,
        })
    }
}

impl BoxEncode for ContentLightLevelBox {
    fn payload_len(&self) -> u64 {
        2 * 2
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u16(self.max_content_light_level)?;
        writer.write_u16(self.max_pic_average_light_level)
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, Error};

    use super::ContentLightLevelBox;
    use crate::encoded;

    /// `clli` of at most 1000 cd/m² for a pixel and 400 cd/m² for a picture
    const CLLI: &[u8] = b"\0\0\0\x0cclli\x03\xe8\x01\x90";

    fn content_light_level() -> ContentLightLevelBox {
        ContentLightLevelBox {
            max_content_light_level: 1000,
            max_pic_average_light_level: 400,
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            ContentLightLevelBox::decode(CLLI),
            Ok((content_light_level(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&content_light_level()), CLLI);
    }

    #[test]
    fn a_payload_ending_inside_a_field_is_refused() {
        assert_eq!(
            ContentLightLevelBox::decode_payload(b"\x03\xe8\x01"),
            Err(Error::truncated_payload(4, 3))
        );
    }
}
