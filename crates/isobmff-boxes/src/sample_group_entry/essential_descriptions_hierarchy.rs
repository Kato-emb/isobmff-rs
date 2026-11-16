//! [`EssentialDescriptionsHierarchyEntry`], the order in which a reader applies the essential sample group descriptions

use alloc::vec::Vec;

use isobmff_core::{Error, FieldReader, FieldWriter, FourCc};

use super::entry::{DerivedEntry, SampleGroupDescriptionEntry, read_class, write_class};

use crate::table::{counted_len, read_counted, write_counted};

/// [`EssentialDescriptionsHierarchyEntry`] (`esgh`), ISO/IEC 14496-12 §10.12
///
/// The order in which a reader applies the essential sample group
/// descriptions, and the transformations of reserved values such as `stsd`
/// and `cenc`, to a sample.
#[doc(alias = "esgh")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct EssentialDescriptionsHierarchyEntry {
    /// `sample_group_description_type` of each, in the order they apply
    pub sample_group_description_types: Vec<FourCc>,
}

impl DerivedEntry for EssentialDescriptionsHierarchyEntry {
    const GROUPING_TYPE: FourCc = FourCc::new(*b"esgh");

    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            sample_group_description_types: read_counted(reader)?,
        })
    }

    fn encoded_len(&self) -> u64 {
        counted_len(&self.sample_group_description_types)
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_counted(writer, &self.sample_group_description_types)
    }
}

impl TryFrom<&SampleGroupDescriptionEntry> for EssentialDescriptionsHierarchyEntry {
    type Error = Error;

    fn try_from(entry: &SampleGroupDescriptionEntry) -> Result<Self, Error> {
        read_class(entry)
    }
}

impl TryFrom<&EssentialDescriptionsHierarchyEntry> for SampleGroupDescriptionEntry {
    type Error = Error;

    fn try_from(entry: &EssentialDescriptionsHierarchyEntry) -> Result<Self, Error> {
        write_class(entry)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{Error, FourCc};

    use super::super::entry::{assert_round_trips, entry_of};
    use super::EssentialDescriptionsHierarchyEntry;

    #[test]
    fn reads_and_writes_the_bytes_its_syntax_lays_out() {
        assert_round_trips(&[(
            b"esgh",
            b"\0\0\0\x02stsdcenc".as_slice(),
            EssentialDescriptionsHierarchyEntry {
                sample_group_description_types: vec![FourCc::new(*b"stsd"), FourCc::new(*b"cenc")],
            },
        )]);
    }

    #[test]
    fn an_entry_of_another_grouping_type_is_refused() {
        assert_eq!(
            [EssentialDescriptionsHierarchyEntry::try_from(&entry_of(
                b"seig",
                b"\0\0\0\x02stsdcenc"
            ))
            .map(|_| ()),],
            [Err(Error::invalid_value("grouping_type")); 1]
        );
    }
}
