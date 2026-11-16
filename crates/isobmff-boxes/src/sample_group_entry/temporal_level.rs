//! [`TemporalLevelEntry`], the temporal level of the samples of a sample group

use isobmff_core::{Error, FieldReader, FieldWriter, FourCc};

use super::entry::{DerivedEntry, SampleGroupDescriptionEntry, read_class, write_class};

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

impl DerivedEntry for TemporalLevelEntry {
    const GROUPING_TYPE: FourCc = FourCc::new(*b"tele");

    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let &[byte] = reader.read_bytes::<1>()?;

        Ok(Self {
            level_independently_decodable: byte & 0x80 != 0,
        })
    }

    fn encoded_len(&self) -> u64 {
        1
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&[if self.level_independently_decodable {
            0x80
        } else {
            0
        }])
    }
}

impl TryFrom<&SampleGroupDescriptionEntry> for TemporalLevelEntry {
    type Error = Error;

    fn try_from(entry: &SampleGroupDescriptionEntry) -> Result<Self, Error> {
        read_class(entry)
    }
}

impl TryFrom<&TemporalLevelEntry> for SampleGroupDescriptionEntry {
    type Error = Error;

    fn try_from(entry: &TemporalLevelEntry) -> Result<Self, Error> {
        write_class(entry)
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
