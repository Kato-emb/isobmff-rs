//! [`TemporalLevelEntry`], the temporal level of the samples of a sample group

use isobmff_core::Error;

use super::entry::{SampleGroupDescriptionEntry, from_class, of_grouping_type, read_whole};

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
    /// How many bytes the entry takes
    pub(super) const ENCODED_LEN: u64 = 1;
}

impl TryFrom<&SampleGroupDescriptionEntry> for TemporalLevelEntry {
    type Error = Error;

    fn try_from(entry: &SampleGroupDescriptionEntry) -> Result<Self, Error> {
        of_grouping_type(entry, b"tele")?;
        read_whole(&entry.data, |reader| {
            let &[byte] = reader.read_bytes::<1>()?;

            Ok(Self {
                level_independently_decodable: byte & 0x80 != 0,
            })
        })
    }
}

impl TryFrom<&TemporalLevelEntry> for SampleGroupDescriptionEntry {
    type Error = Error;

    fn try_from(class: &TemporalLevelEntry) -> Result<Self, Error> {
        from_class(b"tele", TemporalLevelEntry::ENCODED_LEN, |writer| {
            writer.write_bytes(&[if class.level_independently_decodable {
                0x80
            } else {
                0
            }])
        })
    }
}

#[cfg(test)]
mod tests {

    use isobmff_core::Error;

    use super::super::entry::{assert_round_trips, entry_of};
    use super::TemporalLevelEntry;

    #[test]
    fn reads_and_writes_the_bytes_its_syntax_lays_out() {
        assert_round_trips(&[(
            b"tele",
            b"\x80".as_slice(),
            TemporalLevelEntry {
                level_independently_decodable: true,
            },
        )]);
    }

    #[test]
    fn an_entry_of_another_grouping_type_is_refused() {
        assert_eq!(
            [TemporalLevelEntry::try_from(&entry_of(b"seig", b"\x80")).map(|_| ()),],
            [Err(Error::invalid_value("grouping_type")); 1]
        );
    }
}
