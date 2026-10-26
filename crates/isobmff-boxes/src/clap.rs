//! [`CleanApertureBox`], the part of the decoded picture meant for display

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

/// [`CleanApertureBox`] (`clap`), ISO/IEC 14496-12 §12.1.4
///
/// Width, height and offset of the clean aperture, each a fraction of a
/// numerator `N` and a denominator `D`. The offsets are those of the centre
/// of the clean aperture from the centre of the picture, whose horizontal
/// position is `horiz_off` + (width − 1) / 2 for the `width` of the
/// [`VisualSampleEntry`](crate::VisualSampleEntry), and vertical position
/// likewise. The cropping applies before any transformation of the track or
/// movie matrix.
///
/// Every denominator shall be strictly positive, and the numerators of the
/// width and height positive. The fields are signed 32-bit integers, and a
/// value outside these limits is read as found.
#[doc(alias = "clap")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct CleanApertureBox {
    /// Numerator of the width of the clean aperture
    pub clean_aperture_width_n: i32,
    /// Denominator of the width of the clean aperture
    pub clean_aperture_width_d: i32,
    /// Numerator of the height of the clean aperture
    pub clean_aperture_height_n: i32,
    /// Denominator of the height of the clean aperture
    pub clean_aperture_height_d: i32,
    /// Numerator of the horizontal offset of the centre of the clean aperture, typically 0
    pub horiz_off_n: i32,
    /// Denominator of the horizontal offset of the centre of the clean aperture
    pub horiz_off_d: i32,
    /// Numerator of the vertical offset of the centre of the clean aperture, typically 0
    pub vert_off_n: i32,
    /// Denominator of the vertical offset of the centre of the clean aperture
    pub vert_off_d: i32,
}

impl BoxDefinition for CleanApertureBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"clap");
}

impl BoxDecode for CleanApertureBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            clean_aperture_width_n: reader.read_i32()?,
            clean_aperture_width_d: reader.read_i32()?,
            clean_aperture_height_n: reader.read_i32()?,
            clean_aperture_height_d: reader.read_i32()?,
            horiz_off_n: reader.read_i32()?,
            horiz_off_d: reader.read_i32()?,
            vert_off_n: reader.read_i32()?,
            vert_off_d: reader.read_i32()?,
        })
    }
}

impl BoxEncode for CleanApertureBox {
    fn payload_len(&self) -> u64 {
        8 * 4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_i32(self.clean_aperture_width_n)?;
        writer.write_i32(self.clean_aperture_width_d)?;
        writer.write_i32(self.clean_aperture_height_n)?;
        writer.write_i32(self.clean_aperture_height_d)?;
        writer.write_i32(self.horiz_off_n)?;
        writer.write_i32(self.horiz_off_d)?;
        writer.write_i32(self.vert_off_n)?;
        writer.write_i32(self.vert_off_d)
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, Error};

    use super::CleanApertureBox;
    use crate::encoded;

    /// `clap` of 1920 by 1080, its centre a half pixel left of the centre of the picture
    const CLAP: &[u8] = b"\0\0\0\x28clap\
        \0\0\x07\x80\0\0\0\x01\0\0\x04\x38\0\0\0\x01\xff\xff\xff\xff\0\0\0\x02\0\0\0\0\0\0\0\x01";

    fn clean_aperture() -> CleanApertureBox {
        CleanApertureBox {
            clean_aperture_width_n: 1920,
            clean_aperture_width_d: 1,
            clean_aperture_height_n: 1080,
            clean_aperture_height_d: 1,
            horiz_off_n: -1,
            horiz_off_d: 2,
            vert_off_n: 0,
            vert_off_d: 1,
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            CleanApertureBox::decode(CLAP),
            Ok((clean_aperture(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&clean_aperture()), CLAP);
    }

    #[test]
    fn a_payload_ending_inside_a_field_is_refused() {
        assert_eq!(
            CleanApertureBox::decode_payload(CLAP.get(8..38).unwrap()),
            Err(Error::truncated_payload(32, 30))
        );
    }
}
