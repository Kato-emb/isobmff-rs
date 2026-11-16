//! [`SampleToMetadataItemEntry`], the metadata items that apply to the samples of a sample group

use alloc::vec::Vec;

use isobmff_core::{Error, FieldReader, FieldWriter, FourCc};

use crate::table::{counted_len, read_counted, write_counted};

/// [`SampleToMetadataItemEntry`] (`stmi`), ISO/IEC 14496-12 §10.7
///
/// The metadata items that apply to the samples.
#[doc(alias = "stmi")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SampleToMetadataItemEntry {
    /// `handler_type` of the `MetaBox` that holds the items
    pub meta_box_handler_type: FourCc,
    /// `item_id` of every item that applies to the samples
    pub item_ids: Vec<u32>,
}

impl Default for SampleToMetadataItemEntry {
    fn default() -> Self {
        Self {
            meta_box_handler_type: FourCc::new([0; 4]),
            item_ids: Vec::new(),
        }
    }
}

impl SampleToMetadataItemEntry {
    pub(super) fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let &handler_type = reader.read_bytes::<4>()?;

        Ok(Self {
            meta_box_handler_type: FourCc::new(handler_type),
            item_ids: read_counted(reader)?,
        })
    }

    pub(super) fn encoded_len(&self) -> u64 {
        counted_len(&self.item_ids).saturating_add(4)
    }

    pub(super) fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(self.meta_box_handler_type.as_bytes())?;
        write_counted(writer, &self.item_ids)
    }
}
