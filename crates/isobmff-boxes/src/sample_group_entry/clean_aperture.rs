//! [`CleanApertureEntry`], the clean aperture of the samples of a sample group

use isobmff_core::{Error, FieldReader, FieldWriter};

/// [`CleanApertureEntry`] (`casg`), ISO/IEC 14496-12 §10.10
///
/// The clean aperture of the samples of a video track whose aperture
/// changes, as a `CleanApertureBox` states it, each value a fraction of a
/// numerator and a denominator.
#[doc(alias = "casg")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct CleanApertureEntry {
    /// Numerator of the width of the clean aperture, in pixels
    pub clean_aperture_width_n: i32,
    /// Denominator of the width of the clean aperture
    pub clean_aperture_width_d: i32,
    /// Numerator of the height of the clean aperture, in pixels
    pub clean_aperture_height_n: i32,
    /// Denominator of the height of the clean aperture
    pub clean_aperture_height_d: i32,
    /// Numerator of the horizontal offset of the center of the clean aperture
    pub horiz_off_n: i32,
    /// Denominator of the horizontal offset of the center of the clean aperture
    pub horiz_off_d: i32,
    /// Numerator of the vertical offset of the center of the clean aperture
    pub vert_off_n: i32,
    /// Denominator of the vertical offset of the center of the clean aperture
    pub vert_off_d: i32,
}

impl CleanApertureEntry {
    pub(super) fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
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

    pub(super) fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        [
            self.clean_aperture_width_n,
            self.clean_aperture_width_d,
            self.clean_aperture_height_n,
            self.clean_aperture_height_d,
            self.horiz_off_n,
            self.horiz_off_d,
            self.vert_off_n,
            self.vert_off_d,
        ]
        .into_iter()
        .try_for_each(|value| writer.write_i32(value))
    }
}
