//! [`MasteringDisplayColourVolumeBox`], the colour volume of the display a video was mastered on

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

/// [`MasteringDisplayColourVolumeBox`] (`mdcv`), ISO/IEC 14496-12 §12.1.7
///
/// Colour primaries, white point and luminance range of the display the video
/// was mastered on, as the mastering display colour volume SEI message of
/// ITU-T H.265 | ISO/IEC 23008-2 gives them. A value of 0 may code an unknown
/// one, as CTA-861-G provides.
#[doc(alias = "mdcv")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct MasteringDisplayColourVolumeBox {
    /// Normalized x chromaticity coordinate of each of the three colour primaries, in units of 0.00002
    pub display_primaries_x: [u16; 3],
    /// Normalized y chromaticity coordinate of each of the three colour primaries, in units of 0.00002
    pub display_primaries_y: [u16; 3],
    /// Normalized x chromaticity coordinate of the white point, in units of 0.00002
    pub white_point_x: u16,
    /// Normalized y chromaticity coordinate of the white point, in units of 0.00002
    pub white_point_y: u16,
    /// Largest luminance of the display, in units of 0.0001 candelas per square metre
    pub max_display_mastering_luminance: u32,
    /// Smallest luminance of the display, in units of 0.0001 candelas per square metre
    pub min_display_mastering_luminance: u32,
}

impl BoxDefinition for MasteringDisplayColourVolumeBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mdcv");
}

impl BoxDecode for MasteringDisplayColourVolumeBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut display_primaries_x = [0; 3];
        let mut display_primaries_y = [0; 3];
        for (primary_x, primary_y) in display_primaries_x.iter_mut().zip(&mut display_primaries_y) {
            *primary_x = reader.read_u16()?;
            *primary_y = reader.read_u16()?;
        }

        Ok(Self {
            display_primaries_x,
            display_primaries_y,
            white_point_x: reader.read_u16()?,
            white_point_y: reader.read_u16()?,
            max_display_mastering_luminance: reader.read_u32()?,
            min_display_mastering_luminance: reader.read_u32()?,
        })
    }
}

impl BoxEncode for MasteringDisplayColourVolumeBox {
    fn payload_len(&self) -> u64 {
        3 * 4 + 4 + 2 * 4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        for (primary_x, primary_y) in self
            .display_primaries_x
            .iter()
            .zip(&self.display_primaries_y)
        {
            writer.write_u16(*primary_x)?;
            writer.write_u16(*primary_y)?;
        }
        writer.write_u16(self.white_point_x)?;
        writer.write_u16(self.white_point_y)?;
        writer.write_u32(self.max_display_mastering_luminance)?;
        writer.write_u32(self.min_display_mastering_luminance)
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, Error};

    use super::MasteringDisplayColourVolumeBox;
    use crate::encoded;

    /// `mdcv` of the primaries and white point of P3-D65, from 0.005 to 1000 cd/m²
    const MDCV: &[u8] = b"\0\0\0\x20mdcv\
        \x33\xc2\x86\xc4\x1d\x4c\x0b\xb8\x84\xd0\x3e\x80\x3d\x13\x40\x42\0\x98\x96\x80\0\0\0\x32";

    fn mastering_display_colour_volume() -> MasteringDisplayColourVolumeBox {
        MasteringDisplayColourVolumeBox {
            display_primaries_x: [13250, 7500, 34000],
            display_primaries_y: [34500, 3000, 16000],
            white_point_x: 15635,
            white_point_y: 16450,
            max_display_mastering_luminance: 10_000_000,
            min_display_mastering_luminance: 50,
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            MasteringDisplayColourVolumeBox::decode(MDCV),
            Ok((mastering_display_colour_volume(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&mastering_display_colour_volume()), MDCV);
    }

    #[test]
    fn a_payload_ending_inside_a_field_is_refused() {
        assert_eq!(
            MasteringDisplayColourVolumeBox::decode_payload(MDCV.get(8..31).unwrap()),
            Err(Error::truncated_payload(24, 23))
        );
    }
}
