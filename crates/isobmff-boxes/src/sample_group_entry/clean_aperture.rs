//! [`CleanApertureEntry`], the clean aperture of the samples of a sample group

use isobmff_core::{Error, FieldReader, FieldWriter, FourCc};

use super::entry::{DerivedEntry, SampleGroupDescriptionEntry, read_class, write_class};

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

impl DerivedEntry for CleanApertureEntry {
    const GROUPING_TYPE: FourCc = FourCc::new(*b"casg");

    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
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

    fn encoded_len(&self) -> u64 {
        32
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
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

impl TryFrom<&SampleGroupDescriptionEntry> for CleanApertureEntry {
    type Error = Error;

    fn try_from(entry: &SampleGroupDescriptionEntry) -> Result<Self, Error> {
        read_class(entry)
    }
}

impl TryFrom<&CleanApertureEntry> for SampleGroupDescriptionEntry {
    type Error = Error;

    fn try_from(entry: &CleanApertureEntry) -> Result<Self, Error> {
        write_class(entry)
    }
}

#[cfg(test)]
mod tests {

    use isobmff_core::Error;

    use super::super::entry::{assert_round_trips, entry_of};
    use super::CleanApertureEntry;

    #[test]
    fn reads_and_writes_the_bytes_its_syntax_lays_out() {
        assert_round_trips(&[
            (b"casg", b"\0\0\x07\x80\0\0\0\x01\0\0\x04\x38\0\0\0\x01\xff\xff\xff\xf8\0\0\0\x01\0\0\0\0\0\0\0\x01".as_slice(), CleanApertureEntry { clean_aperture_width_n: 1920, clean_aperture_width_d: 1, clean_aperture_height_n: 1080, clean_aperture_height_d: 1, horiz_off_n: -8, horiz_off_d: 1, vert_off_n: 0, vert_off_d: 1 }),
        ]);
    }

    #[test]
    fn an_entry_of_another_grouping_type_is_refused() {
        assert_eq!(
            [
                CleanApertureEntry::try_from(&entry_of(b"seig", b"\0\0\x07\x80\0\0\0\x01\0\0\x04\x38\0\0\0\x01\xff\xff\xff\xf8\0\0\0\x01\0\0\0\0\0\0\0\x01")).map(|_| ()),
            ],
            [Err(Error::invalid_value("grouping_type")); 1]
        );
    }
}
