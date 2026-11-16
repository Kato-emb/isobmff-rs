//! [`TemporalLevelEntry`], the temporal level of the samples of a sample group

use isobmff_core::{Error, FieldReader, FieldWriter};

/// [`TemporalLevelEntry`] (`tele`), ISO/IEC 14496-12 §10.5
///
/// A temporal level, which is the index of the entry: its samples depend on
/// none of a higher level.
#[doc(alias = "tele")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct TemporalLevelEntry {
    /// Whether the samples of the level depend on none of other levels,
    /// `false` for no information
    pub level_independently_decodable: bool,
}

impl TemporalLevelEntry {
    pub(super) fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let &[byte] = reader.read_bytes::<1>()?;

        Ok(Self {
            level_independently_decodable: byte & 0x80 != 0,
        })
    }

    pub(super) fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&[if self.level_independently_decodable {
            0x80
        } else {
            0
        }])
    }
}
