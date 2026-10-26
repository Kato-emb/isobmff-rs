//! [`PixelAspectRatioBox`], the aspect ratio of a pixel of the video

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

/// [`PixelAspectRatioBox`] (`pasp`), ISO/IEC 14496-12 §12.1.4
///
/// Relative width and height of a pixel: a pixel that appears H wide and V
/// tall has `h_spacing` / `v_spacing` equal to H / V. The units are
/// unspecified, and only the ratio matters. Where present, the box overrides
/// what structures of the video codec declare.
///
/// Both values shall be strictly positive. A value of 0 is read as found.
#[doc(alias = "pasp")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct PixelAspectRatioBox {
    /// Relative width of a pixel
    pub h_spacing: u32,
    /// Relative height of a pixel
    pub v_spacing: u32,
}

impl BoxDefinition for PixelAspectRatioBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"pasp");
}

impl BoxDecode for PixelAspectRatioBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            h_spacing: reader.read_u32()?,
            v_spacing: reader.read_u32()?,
        })
    }
}

impl BoxEncode for PixelAspectRatioBox {
    fn payload_len(&self) -> u64 {
        2 * 4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(self.h_spacing)?;
        writer.write_u32(self.v_spacing)
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, Error};

    use super::PixelAspectRatioBox;
    use crate::encoded;

    /// `pasp` of pixels 4 wide for 3 tall
    const PASP: &[u8] = b"\0\0\0\x10pasp\0\0\0\x04\0\0\0\x03";

    fn pixel_aspect_ratio() -> PixelAspectRatioBox {
        PixelAspectRatioBox {
            h_spacing: 4,
            v_spacing: 3,
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            PixelAspectRatioBox::decode(PASP),
            Ok((pixel_aspect_ratio(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&pixel_aspect_ratio()), PASP);
    }

    #[test]
    fn a_payload_ending_inside_a_field_is_refused() {
        assert_eq!(
            PixelAspectRatioBox::decode_payload(b"\0\0\0\x04\0\0"),
            Err(Error::truncated_payload(8, 6))
        );
    }
}
