//! What the data offsets of the samples count from, their `data origin`
//!
//! A data offset counts from a data origin, ISO/IEC 14496-12 §6.1 Table 1:
//! for the samples of a sample table, the start of the file the data
//! reference entry of their sample entry identifies, or the payload of an
//! `IdentifiedMediaDataBox`, §8.7.5.3.

use isobmff_boxes::{DataEntry, DataReferenceBox, SampleDescriptionBox};

use super::data_reference::data_entry;
use crate::DataOrigin;
use crate::structure_data::Error;

/// Returns what the data offsets of the samples of a sample table whose sample entry is `sample_description_index` count from
///
/// They count from the payload of an `IdentifiedMediaDataBox` where the data
/// reference entry is a `DataEntryImdaBox`, and from the start of the file
/// it identifies otherwise, ISO/IEC 14496-12 §8.7.5.3: "Otherwise, the value
/// of `chunk_offset` is relative to the start of the containing media file".
///
/// # Errors
///
/// * The failures of [`data_entry`].
/// * [`SequenceNumberOutsideMovieFragment`](Error::SequenceNumberOutsideMovieFragment):
///   the data reference entry is a `DataEntrySeqNumImdaBox`, whose samples
///   are each in a movie fragment, §8.7.2.1.
pub(in crate::structure_data) fn sample_table_data_origin(
    stsd: &SampleDescriptionBox,
    dref: &DataReferenceBox,
    sample_description_index: u32,
) -> Result<DataOrigin, Error> {
    match data_entry(stsd, dref, sample_description_index)? {
        DataEntry::Imda(_) => Ok(DataOrigin::IdentifiedMediaData),
        DataEntry::SeqNumImda(_) => Err(Error::SequenceNumberOutsideMovieFragment {
            sample_description_index,
        }),
        DataEntry::Url(_) | DataEntry::Urn(_) | DataEntry::Other(_) | _ => Ok(DataOrigin::File),
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec::Vec;

    use isobmff_boxes::{
        DataEntry, DataEntryImdaBox, DataEntrySeqNumImdaBox, DataEntryUrlBox, DataEntryUrnBox,
        DataReferenceBox, SampleDescription, SampleDescriptionBox, SampleEntry,
    };
    use isobmff_core::{AnyBox, BoxType, FourCc, Utf8String};

    use crate::DataOrigin;

    use super::{Error, sample_table_data_origin};

    /// `stsd` of one sample entry, and `dref` of `data_entry` alone
    fn track_of(data_entry: DataEntry) -> (SampleDescriptionBox, DataReferenceBox) {
        let mut stsd = SampleDescriptionBox::default();
        stsd.sample_descriptions = [SampleDescription::Typed(SampleEntry::new(
            FourCc::new(*b"avc1"),
            1,
            Vec::new(),
        ))]
        .to_vec();
        let mut dref = DataReferenceBox::default();
        dref.data_entries = [data_entry].to_vec();
        (stsd, dref)
    }

    #[test]
    fn the_samples_of_a_sample_table_count_from_an_imda_or_from_the_start_of_the_file_otherwise() {
        let origin = |data_entry| {
            let (stsd, dref) = track_of(data_entry);
            sample_table_data_origin(&stsd, &dref, 1)
        };
        let mut url = DataEntryUrlBox::default();
        url.location = Some(Utf8String::new(String::from("a.mp4")).unwrap());
        let mut urn = DataEntryUrnBox::default();
        urn.name = Utf8String::new(String::from("urn:x")).unwrap();
        let mut imdt = DataEntryImdaBox::default();
        imdt.imda_ref_identifier = 7;
        let other = AnyBox::from_raw_bytes(BoxType::compact(*b"xxxx"), Vec::new());
        assert_eq!(
            [
                origin(DataEntry::Url(DataEntryUrlBox::default())),
                origin(DataEntry::Url(url)),
                origin(DataEntry::Urn(urn)),
                origin(DataEntry::Other(other)),
                origin(DataEntry::Imda(imdt)),
                origin(DataEntry::SeqNumImda(DataEntrySeqNumImdaBox::default())),
            ],
            [
                Ok(DataOrigin::File),
                Ok(DataOrigin::File),
                Ok(DataOrigin::File),
                Ok(DataOrigin::File),
                Ok(DataOrigin::IdentifiedMediaData),
                Err(Error::SequenceNumberOutsideMovieFragment {
                    sample_description_index: 1
                }),
            ]
        );
        let (stsd, dref) = track_of(DataEntry::Url(DataEntryUrlBox::default()));
        assert_eq!(
            sample_table_data_origin(&stsd, &dref, 2),
            Err(Error::SampleDescriptionIndexOutOfRange {
                sample_description_index: 2,
                sample_entry_count: 1
            })
        );
    }
}
