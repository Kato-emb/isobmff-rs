//! [`PixelAspectRatioEntry`], the pixel aspect ratio of the samples of a sample group

use isobmff_core::Error;

use super::entry::{SampleGroupDescriptionEntry, from_class, of_grouping_type, read_whole};

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
    /// How many bytes the entry takes
    pub(super) const ENCODED_LEN: u64 = 8;
}

impl TryFrom<&SampleGroupDescriptionEntry> for PixelAspectRatioEntry {
    type Error = Error;

    fn try_from(entry: &SampleGroupDescriptionEntry) -> Result<Self, Error> {
        of_grouping_type(entry, b"pasr")?;
        read_whole(&entry.data, |reader| {
            Ok(Self {
                h_spacing: reader.read_u32()?,
                v_spacing: reader.read_u32()?,
            })
        })
    }
}

impl TryFrom<&PixelAspectRatioEntry> for SampleGroupDescriptionEntry {
    type Error = Error;

    fn try_from(class: &PixelAspectRatioEntry) -> Result<Self, Error> {
        from_class(b"pasr", PixelAspectRatioEntry::ENCODED_LEN, |writer| {
            writer.write_u32(class.h_spacing)?;
            writer.write_u32(class.v_spacing)
        })
    }
}

#[cfg(test)]
mod tests {

    use isobmff_core::Error;

    use super::super::entry::{assert_round_trips, entry_of};
    use super::PixelAspectRatioEntry;

    #[test]
    fn reads_and_writes_the_bytes_its_syntax_lays_out() {
        assert_round_trips(&[(
            b"pasr",
            b"\0\0\0\x04\0\0\0\x03".as_slice(),
            PixelAspectRatioEntry {
                h_spacing: 4,
                v_spacing: 3,
            },
        )]);
    }

    #[test]
    fn an_entry_of_another_grouping_type_is_refused() {
        assert_eq!(
            [
                PixelAspectRatioEntry::try_from(&entry_of(b"seig", b"\0\0\0\x04\0\0\0\x03"))
                    .map(|_| ()),
            ],
            [Err(Error::invalid_value("grouping_type")); 1]
        );
    }
}
