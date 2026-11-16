//! [`EssentialDescriptionsHierarchyEntry`], the order in which a reader applies the essential sample group descriptions

use alloc::vec::Vec;

use isobmff_core::{Error, FieldReader, FieldWriter, FourCc};

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

impl EssentialDescriptionsHierarchyEntry {
    pub(super) fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            sample_group_description_types: read_counted(reader)?,
        })
    }

    pub(super) fn encoded_len(&self) -> u64 {
        counted_len(&self.sample_group_description_types)
    }

    pub(super) fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_counted(writer, &self.sample_group_description_types)
    }
}
