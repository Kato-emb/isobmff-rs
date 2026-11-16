//! [`PixelAspectRatioEntry`], the pixel aspect ratio of the samples of a sample group

use isobmff_core::{Error, FieldReader, FieldWriter};

/// [`PixelAspectRatioEntry`] (`pasr`), ISO/IEC 14496-12 §10.9
///
/// The pixel aspect ratio of the samples of a video track whose ratio
/// changes, as a `PixelAspectRatioBox` states it.
#[doc(alias = "pasr")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct PixelAspectRatioEntry {
    /// Relative width of a pixel
    pub h_spacing: u32,
    /// Relative height of a pixel
    pub v_spacing: u32,
}

impl PixelAspectRatioEntry {
    pub(super) fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            h_spacing: reader.read_u32()?,
            v_spacing: reader.read_u32()?,
        })
    }

    pub(super) fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(self.h_spacing)?;
        writer.write_u32(self.v_spacing)
    }
}
