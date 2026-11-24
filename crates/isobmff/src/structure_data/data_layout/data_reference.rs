//! Which data reference entry specifies the container of the samples of a sample entry, and which container that is
//!
//! The data reference table (`dref`), ISO/IEC 14496-12 §8.7.2, lists the
//! containers of the media data of a track; the `data_reference_index` of a
//! sample entry names one of them. The type of that entry, and its flags,
//! tell which: the same file, a resource a URL or a URN identifies, or an
//! `IdentifiedMediaDataBox`, §6.1. The data offsets of the samples of a
//! sample table count from the start of that container, §8.7.5.3.

use isobmff_boxes::{
    DataEntry, DataEntryImdaBox, DataEntrySeqNumImdaBox, DataReferenceBox, SampleDescriptionBox,
};
use isobmff_core::BoxDefinition;

use crate::MediaDataContainer;
use crate::structure_data::Error;

/// Returns the data reference entry the sample entry `sample_description_index`, from 1, of `stsd` names in `dref`
///
/// The data reference entry specifies the container of the data of the
/// samples the sample entry describes, ISO/IEC 14496-12 §6.1: the same
/// file, a resource a URL or a URN names, or an `IdentifiedMediaDataBox`.
///
/// ```
/// use isobmff::boxes::{
///     DataEntry, DataEntryUrlBox, DataReferenceBox, SampleDescription, SampleDescriptionBox,
///     SampleEntry,
/// };
/// use isobmff::FourCc;
/// use isobmff::structure_data::data_layout::data_entry;
///
/// // One sample entry, whose samples lie in the same file
/// let mut stsd = SampleDescriptionBox::default();
/// stsd.sample_descriptions = vec![SampleDescription::Typed(SampleEntry::new(
///     FourCc::new(*b"avc1"),
///     1,
///     Vec::new(),
/// ))];
/// let mut dref = DataReferenceBox::default();
/// dref.data_entries = vec![DataEntry::Url(DataEntryUrlBox::default())];
/// assert!(data_entry(&stsd, &dref, 1)?.is_in_same_file());
/// # Ok::<(), isobmff::structure_data::Error>(())
/// ```
///
/// # Errors
///
/// * [`SampleDescriptionIndexOutOfRange`](Error::SampleDescriptionIndexOutOfRange):
///   `sample_description_index` is 0, or past the entries of `stsd`.
/// * [`DataReferenceIndexMissing`](Error::DataReferenceIndexMissing): the
///   sample entry ends before its `data_reference_index`.
/// * [`DataReferenceIndexOutOfRange`](Error::DataReferenceIndexOutOfRange):
///   the `data_reference_index` is 0, or past the entries of `dref`.
pub fn data_entry<'a>(
    stsd: &SampleDescriptionBox,
    dref: &'a DataReferenceBox,
    sample_description_index: u32,
) -> Result<&'a DataEntry, Error> {
    indexed_data_entry(stsd, dref, sample_description_index).map(|(_, data_entry)| data_entry)
}

/// Returns the data reference entry the sample entry `sample_description_index` of `stsd` names in `dref`, with the `data_reference_index` that names it
///
/// # Errors
///
/// * The failures of [`data_entry`].
fn indexed_data_entry<'a>(
    stsd: &SampleDescriptionBox,
    dref: &'a DataReferenceBox,
    sample_description_index: u32,
) -> Result<(u16, &'a DataEntry), Error> {
    let sample_entry = usize::try_from(sample_description_index)
        .ok()
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| stsd.sample_descriptions.get(index))
        .ok_or(Error::SampleDescriptionIndexOutOfRange {
            sample_description_index,
            sample_entry_count: u32::try_from(stsd.sample_descriptions.len()).unwrap_or(u32::MAX),
        })?;
    let data_reference_index =
        sample_entry
            .data_reference_index()
            .ok_or(Error::DataReferenceIndexMissing {
                sample_description_index,
            })?;
    let data_entry = usize::from(data_reference_index)
        .checked_sub(1)
        .and_then(|index| dref.data_entries.get(index))
        .ok_or(Error::DataReferenceIndexOutOfRange {
            sample_description_index,
            data_reference_index,
            data_entry_count: u32::try_from(dref.data_entries.len()).unwrap_or(u32::MAX),
        })?;
    Ok((data_reference_index, data_entry))
}

/// Returns the container the data reference entry of the sample entry `sample_description_index` specifies for the samples of a sample table it describes
///
/// A `DataEntryUrlBox` without a location places them in the same file, and
/// any other `DataEntryUrlBox` or `DataEntryUrnBox` in the resource its URL
/// or URN identifies, ISO/IEC 14496-12 §6.1 and §8.7.2; a `DataEntryImdaBox`
/// in the `IdentifiedMediaDataBox` of its `imda_ref_identifier`, §8.7.2.1;
/// and an entry of a type this crate does not read in a resource other than
/// the same file. A resource is named by the `data_reference_index` of the
/// sample entry, §8.5.2.3.
///
/// # Errors
///
/// * The failures of [`data_entry`].
/// * [`SequenceNumberOutsideMovieFragment`](Error::SequenceNumberOutsideMovieFragment):
///   the data reference entry is a `DataEntrySeqNumImdaBox`, whose samples
///   are each in a movie fragment, §8.7.2.1.
/// * [`DataEntryNotDecoded`](Error::DataEntryNotDecoded): the data reference
///   entry is a `DataEntryImdaBox` or a `DataEntrySeqNumImdaBox` that does
///   not decode.
pub(in crate::structure_data) fn sample_table_container(
    stsd: &SampleDescriptionBox,
    dref: &DataReferenceBox,
    sample_description_index: u32,
) -> Result<MediaDataContainer, Error> {
    let (data_reference_index, data_entry) =
        indexed_data_entry(stsd, dref, sample_description_index)?;
    match data_entry {
        DataEntry::Imda(imdt) => Ok(MediaDataContainer::IdentifiedMediaData {
            imda_identifier: imdt.imda_ref_identifier,
        }),
        DataEntry::SeqNumImda(_) => Err(Error::SequenceNumberOutsideMovieFragment {
            sample_description_index,
        }),
        DataEntry::Other(other)
            if other.box_type() == DataEntryImdaBox::BOX_TYPE
                || other.box_type() == DataEntrySeqNumImdaBox::BOX_TYPE =>
        {
            Err(Error::DataEntryNotDecoded {
                sample_description_index,
                box_type: other.box_type(),
            })
        }
        same_file if same_file.is_in_same_file() => Ok(MediaDataContainer::SameFile),
        DataEntry::Url(_) | DataEntry::Urn(_) | DataEntry::Other(_) | _ => {
            Ok(MediaDataContainer::Resource {
                data_reference_index,
            })
        }
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
    use isobmff_core::{AnyBox, BoxDecode, BoxType, FourCc, Utf8String};

    use crate::MediaDataContainer;

    use super::{Error, sample_table_container};

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
    fn the_samples_of_a_sample_table_lie_in_the_container_their_entry_specifies() {
        let container = |data_entry| {
            let (stsd, dref) = track_of(data_entry);
            sample_table_container(&stsd, &dref, 1)
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
                container(DataEntry::Url(DataEntryUrlBox::default())),
                container(DataEntry::Url(url)),
                container(DataEntry::Urn(urn)),
                container(DataEntry::Other(other)),
                container(DataEntry::Imda(imdt)),
                container(DataEntry::SeqNumImda(DataEntrySeqNumImdaBox::default())),
            ],
            [
                Ok(MediaDataContainer::SameFile),
                Ok(MediaDataContainer::Resource {
                    data_reference_index: 1
                }),
                Ok(MediaDataContainer::Resource {
                    data_reference_index: 1
                }),
                Ok(MediaDataContainer::Resource {
                    data_reference_index: 1
                }),
                Ok(MediaDataContainer::IdentifiedMediaData { imda_identifier: 7 }),
                Err(Error::SequenceNumberOutsideMovieFragment {
                    sample_description_index: 1
                }),
            ]
        );
        let (stsd, dref) = track_of(DataEntry::Url(DataEntryUrlBox::default()));
        assert_eq!(
            sample_table_container(&stsd, &dref, 2),
            Err(Error::SampleDescriptionIndexOutOfRange {
                sample_description_index: 2,
                sample_entry_count: 1
            })
        );
    }

    #[test]
    fn a_data_reference_entry_of_an_identified_media_data_that_does_not_decode_is_refused() {
        for (box_type, payload) in [
            (*b"imdt", [1, 0, 0, 0, 0, 0, 0, 7].as_slice()),
            (*b"snim", [1, 0, 0, 0].as_slice()),
        ] {
            let size = u32::try_from(payload.len() + 8).unwrap();
            let dref = [
                &[0, 0, 0, 0][..],
                &1_u32.to_be_bytes(),
                &size.to_be_bytes(),
                &box_type,
                payload,
            ]
            .concat();
            let dref = DataReferenceBox::decode_payload(&dref).unwrap();
            let box_type = BoxType::compact(box_type);
            assert_eq!(
                dref.data_entries,
                [DataEntry::Other(AnyBox::from_raw_bytes(
                    box_type,
                    payload.to_vec()
                ))]
            );
            let (stsd, _) = track_of(DataEntry::Url(DataEntryUrlBox::default()));
            assert_eq!(
                sample_table_container(&stsd, &dref, 1),
                Err(Error::DataEntryNotDecoded {
                    sample_description_index: 1,
                    box_type
                })
            );
        }
    }
}
