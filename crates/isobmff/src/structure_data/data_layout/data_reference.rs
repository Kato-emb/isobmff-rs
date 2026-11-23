//! Which data reference entry specifies the container of the samples of a sample entry, and what their data offsets count from
//!
//! The data reference table (`dref`), ISO/IEC 14496-12 §8.7.2, lists the
//! containers of the media data of a track; the `data_reference_index` of a
//! sample entry names one of them. [`data_entry`] looks that entry up.

use isobmff_boxes::{DataEntry, DataReferenceBox, SampleDescriptionBox};

use crate::DataOrigin;
use crate::structure_data::Error;

/// Returns the number of the entries of a table of `length` entries, or `u32::MAX` past what a `u32` counts, the width of an `entry_count`
pub(super) fn count(length: usize) -> u32 {
    u32::try_from(length).unwrap_or(u32::MAX)
}

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
            sample_entry_count: count(stsd.sample_descriptions.len()),
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
            data_entry_count: count(dref.data_entries.len()),
        })
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
