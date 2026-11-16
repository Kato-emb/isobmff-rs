//! [`SampleToMetadataItemEntry`], the metadata items that apply to the samples of a sample group

use alloc::vec::Vec;

use isobmff_core::{Error, FourCc};

use super::entry::{SampleGroupDescriptionEntry, from_class, of_grouping_type, read_whole};

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

impl TryFrom<&SampleGroupDescriptionEntry> for SampleToMetadataItemEntry {
    type Error = Error;

    fn try_from(entry: &SampleGroupDescriptionEntry) -> Result<Self, Error> {
        of_grouping_type(entry, b"stmi")?;
        read_whole(&entry.data, |reader| {
            let &handler_type = reader.read_bytes::<4>()?;

            Ok(Self {
                meta_box_handler_type: FourCc::new(handler_type),
                item_ids: read_counted(reader)?,
            })
        })
    }
}

impl TryFrom<&SampleToMetadataItemEntry> for SampleGroupDescriptionEntry {
    type Error = Error;

    fn try_from(class: &SampleToMetadataItemEntry) -> Result<Self, Error> {
        from_class(
            b"stmi",
            counted_len(&class.item_ids).saturating_add(4),
            |writer| {
                writer.write_bytes(class.meta_box_handler_type.as_bytes())?;
                write_counted(writer, &class.item_ids)
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{Error, FourCc};

    use super::super::entry::{assert_round_trips, entry_of};
    use super::SampleToMetadataItemEntry;

    #[test]
    fn reads_and_writes_the_bytes_its_syntax_lays_out() {
        assert_round_trips(&[(
            b"stmi",
            b"meta\0\0\0\x02\0\0\0\x01\0\0\0\x07".as_slice(),
            SampleToMetadataItemEntry {
                meta_box_handler_type: FourCc::new(*b"meta"),
                item_ids: vec![1, 7],
            },
        )]);
    }

    #[test]
    fn an_entry_of_another_grouping_type_is_refused() {
        assert_eq!(
            [SampleToMetadataItemEntry::try_from(&entry_of(
                b"seig",
                b"meta\0\0\0\x02\0\0\0\x01\0\0\0\x07"
            ))
            .map(|_| ()),],
            [Err(Error::invalid_value("grouping_type")); 1]
        );
    }
}
