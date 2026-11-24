//! Which data reference entry specifies the container of the samples of a sample entry, and the `data origin` of the samples of a sample table
//!
//! The data reference table (`dref`), ISO/IEC 14496-12 §8.7.2, lists the
//! containers of the media data of a track; the `data_reference_index` of a
//! sample entry names one of them. The type of that entry sets the data
//! origin of the samples of a sample table, §8.7.5.3.

use isobmff_boxes::{
    DataEntry, DataEntryImdaBox, DataEntrySeqNumImdaBox, DataReferenceBox, SampleDescriptionBox,
};
use isobmff_core::BoxDefinition;

use crate::DataOrigin;
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
    usize::from(data_reference_index)
        .checked_sub(1)
        .and_then(|index| dref.data_entries.get(index))
        .ok_or(Error::DataReferenceIndexOutOfRange {
            sample_description_index,
            data_reference_index,
            data_entry_count: u32::try_from(dref.data_entries.len()).unwrap_or(u32::MAX),
        })
}

/// Returns the `imda_identifier` of the `IdentifiedMediaDataBox` the data reference entry `data_entry` of the sample entry `sample_description_index` places the media data in, or `None` where it places it in none
///
/// A `DataEntryImdaBox` names the `IdentifiedMediaDataBox` by its
/// `imda_ref_identifier`, and a `DataEntrySeqNumImdaBox` by the
/// `sequence_number` of the movie fragment that contains the sample,
/// ISO/IEC 14496-12 §8.7.2.1; `sequence_number` is that of the movie fragment
/// of the sample, or `None` for a sample in no movie fragment. Any other
/// entry places the media data in the file it identifies.
///
/// # Errors
///
/// * [`SequenceNumberOutsideMovieFragment`](Error::SequenceNumberOutsideMovieFragment):
///   `data_entry` is a `DataEntrySeqNumImdaBox`, and the sample is in no movie
///   fragment, §8.7.2.1.
/// * [`DataEntryNotDecoded`](Error::DataEntryNotDecoded): `data_entry` is a
///   `DataEntryImdaBox` or a `DataEntrySeqNumImdaBox` that does not decode.
pub(super) fn imda_identifier(
    data_entry: &DataEntry,
    sample_description_index: u32,
    sequence_number: Option<u32>,
) -> Result<Option<u32>, Error> {
    match data_entry {
        DataEntry::Imda(imdt) => Ok(Some(imdt.imda_ref_identifier)),
        DataEntry::SeqNumImda(_) => {
            sequence_number
                .map(Some)
                .ok_or(Error::SequenceNumberOutsideMovieFragment {
                    sample_description_index,
                })
        }
        DataEntry::Other(other)
            if other.box_type() == DataEntryImdaBox::BOX_TYPE
                || other.box_type() == DataEntrySeqNumImdaBox::BOX_TYPE =>
        {
            Err(Error::DataEntryNotDecoded {
                sample_description_index,
                box_type: other.box_type(),
            })
        }
        DataEntry::Url(_) | DataEntry::Urn(_) | DataEntry::Other(_) | _ => Ok(None),
    }
}

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
/// * [`DataEntryNotDecoded`](Error::DataEntryNotDecoded): the data reference
///   entry is a `DataEntryImdaBox` or a `DataEntrySeqNumImdaBox` that does
///   not decode.
pub(in crate::structure_data) fn sample_table_data_origin(
    stsd: &SampleDescriptionBox,
    dref: &DataReferenceBox,
    sample_description_index: u32,
) -> Result<DataOrigin, Error> {
    let data_entry = data_entry(stsd, dref, sample_description_index)?;
    Ok(
        match imda_identifier(data_entry, sample_description_index, None)? {
            Some(_) => DataOrigin::IdentifiedMediaData,
            None => DataOrigin::File,
        },
    )
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

    use crate::DataOrigin;

    use super::{Error, imda_identifier, sample_table_data_origin};

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

    #[test]
    fn an_entry_names_its_imda_by_identifier_or_by_the_sequence_number_of_its_movie_fragment() {
        let mut imdt = DataEntryImdaBox::default();
        imdt.imda_ref_identifier = 7;
        let (imdt, snim, url) = (
            DataEntry::Imda(imdt),
            DataEntry::SeqNumImda(DataEntrySeqNumImdaBox::default()),
            DataEntry::Url(DataEntryUrlBox::default()),
        );
        assert_eq!(
            [
                imda_identifier(&imdt, 1, None),
                imda_identifier(&imdt, 1, Some(3)),
                imda_identifier(&snim, 1, Some(3)),
                imda_identifier(&url, 1, Some(3)),
                imda_identifier(&snim, 2, None),
            ],
            [
                Ok(Some(7)),
                Ok(Some(7)),
                Ok(Some(3)),
                Ok(None),
                Err(Error::SequenceNumberOutsideMovieFragment {
                    sample_description_index: 2
                }),
            ]
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
                sample_table_data_origin(&stsd, &dref, 1),
                Err(Error::DataEntryNotDecoded {
                    sample_description_index: 1,
                    box_type
                })
            );
        }
    }
}
