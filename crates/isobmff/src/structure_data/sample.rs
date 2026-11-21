//! [`TrackSampleDescription`], and what the writers of structure-data hold of a sample and check of it on input

use alloc::vec::Vec;

use isobmff_boxes::{DataEntry, DataReferenceBox, SampleDescriptionBox, SampleFlags};

use isobmff_core::BoxDefinition;

use super::Error;
use crate::SampleLocation;

/// The sample entries of a track and the data references they name
///
/// The [`SampleDescriptionBox`] (`stsd`), ISO/IEC 14496-12 §8.5.2, and the
/// [`DataReferenceBox`] (`dref`), §8.7.2, of a track, which a writer of
/// structure-data checks each sample against: its `sample_description_index`
/// names a sample entry, and the entry places the media data in the same file.
///
/// ```
/// use isobmff::structure_data::TrackSampleDescription;
/// use isobmff_boxes::{DataReferenceBox, SampleDescriptionBox};
///
/// let (stsd, dref) = (SampleDescriptionBox::default(), DataReferenceBox::default());
/// let track = TrackSampleDescription { track_id: 1, stsd: &stsd, dref: &dref };
/// assert_eq!(track.stsd.sample_descriptions.len(), 0);
/// ```
#[expect(
    clippy::exhaustive_structs,
    reason = "a caller of a writer builds one, and a track is described by its track_ID, its sample entries and its data references"
)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackSampleDescription<'a> {
    /// `track_ID` of the track
    pub track_id: u32,
    /// The sample entries of the track
    pub stsd: &'a SampleDescriptionBox,
    /// The data references the sample entries name
    pub dref: &'a DataReferenceBox,
}

/// What a writer of structure-data holds of a sample
#[derive(Debug, Clone, Copy)]
pub(super) struct Sample {
    /// Offset of the sample, from where the writer counts
    pub(super) offset: u64,
    pub(super) size: u32,
    pub(super) sample_description_index: u32,
    pub(super) sample_duration: u32,
    pub(super) sample_composition_time_offset: i32,
    pub(super) sample_flags: SampleFlags,
}

/// Where the data reference entry of a sample entry places the media data
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MediaData {
    /// In the same file, ISO/IEC 14496-12 §8.7.2.1
    SameFile,
    /// In the `imda` of this `imda_identifier`: a `DataEntryImdaBox`
    Identified(u32),
    /// In the `imda` whose `imda_identifier` is the `sequence_number` of the
    /// movie fragment of each sample: a `DataEntrySeqNumImdaBox`
    SequenceNumber,
    /// In another file: the one a `DataEntryUrlBox` with a `location` or a
    /// `DataEntryUrnBox` names, or one an entry of a type this crate does not
    /// read names
    Elsewhere,
}

/// The `data_reference_index` of a sample entry and where the data reference entry it names places the media data, as [`entry_data_references`] reads them
#[derive(Debug, Clone, Copy)]
pub(super) struct EntryDataReference(
    // Why not hold the error of the entry: it is many times the size of
    // what it is built from, and a reader holds an entry for each sample
    // entry of each track.
    /// `None` for an entry that ends before its `data_reference_index`, and
    /// a `MediaData` of `None` for an index that names no data reference
    /// entry
    Option<(u16, Option<MediaData>)>,
);

/// The `data_reference_index` of each sample entry of `stsd`, from 1, and where the entry of `dref` it names places the media data
pub(super) fn entry_data_references(
    stsd: &SampleDescriptionBox,
    dref: &DataReferenceBox,
) -> Vec<EntryDataReference> {
    stsd.sample_descriptions
        .iter()
        .map(|entry| {
            EntryDataReference(entry.data_reference_index().map(|data_reference_index| {
                let media_data = match usize::from(data_reference_index)
                    .checked_sub(1)
                    .and_then(|index| dref.data_entries.get(index))
                {
                    Some(entry) if entry.is_in_same_file() => Some(MediaData::SameFile),
                    Some(DataEntry::Imda(imdt)) => {
                        Some(MediaData::Identified(imdt.imda_ref_identifier))
                    }
                    Some(DataEntry::SeqNumImda(_)) => Some(MediaData::SequenceNumber),
                    Some(_) => Some(MediaData::Elsewhere),
                    None => None,
                };
                (data_reference_index, media_data)
            }))
        })
        .collect()
}

/// The `data_reference_index` of the sample entry `sample_description_index`, from 1, of the entries of track `track_id`, and where it places the media data
///
/// A sample entry is checked here, where a sample names it, rather than
/// where the entries are read: one no sample names is not refused.
///
/// # Errors
/// * [`Error::SampleDescriptionIndexOutOfRange`]: the track has no such
///   sample entry.
/// * [`Error::InvalidEntry`]: the sample entry ends before its
///   `data_reference_index`.
/// * [`Error::DataReferenceIndexOutOfRange`]: the `data_reference_index` is
///   0, or past the entries of the `dref`, which ISO/IEC 14496-12 §8.5.2.3
///   numbers from 1.
pub(super) fn data_reference(
    entries: &[EntryDataReference],
    track_id: u32,
    sample_description_index: u32,
) -> Result<(u16, MediaData), Error> {
    let entry = usize::try_from(sample_description_index)
        .ok()
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| entries.get(index))
        .ok_or(Error::SampleDescriptionIndexOutOfRange {
            sample_description_index,
        })?;
    let (data_reference_index, media_data) = entry.0.ok_or(Error::InvalidEntry {
        track_id,
        box_type: SampleDescriptionBox::BOX_TYPE,
        entry: u64::from(sample_description_index.saturating_sub(1)),
    })?;
    let media_data = media_data.ok_or(Error::DataReferenceIndexOutOfRange {
        track_id,
        data_reference_index,
    })?;
    Ok((data_reference_index, media_data))
}

/// Returns what a writer holds of the sample at `location`, which joins `held` samples under `sample_count_limit`, its sample entry one of `entries`
///
/// # Errors
/// * [`Error::SampleCountOverLimit`]: the writer holds as many samples as
///   the limit, or as a table or a track fragment run counts.
/// * [`Error::SampleDescriptionIndexOutOfRange`]: the sample names no entry
///   of `entries`.
/// * [`Error::MediaDataElsewhere`]: the entry places the media data in
///   another file, or in an `IdentifiedMediaDataBox`.
/// * [`Error::DataReferenceIndexOutOfRange`]: the sample entry names no data
///   reference entry.
/// * [`Error::InvalidEntry`]: the sample entry ends before its
///   `data_reference_index`.
/// * [`Error::Overflow`]: the bytes of the sample end past the last offset
///   there can be.
/// * [`Error::CompositionOffsetOutOfRange`]: the composition offset of the
///   sample does not fit 32 bits, signed.
pub(super) fn checked(
    location: &SampleLocation,
    entries: &[EntryDataReference],
    held: usize,
    sample_count_limit: u64,
) -> Result<Sample, Error> {
    let limit = sample_count_limit.min(u64::from(u32::MAX));
    let sample_count = u64::try_from(held).unwrap_or(u64::MAX).saturating_add(1);
    if sample_count > limit {
        return Err(Error::SampleCountOverLimit {
            sample_count,
            limit,
        });
    }
    let properties = location.properties;
    let track_id = properties.track_id;
    let sample_description_index = properties.sample_description_index;
    match data_reference(entries, track_id, sample_description_index)? {
        (_, MediaData::SameFile) => {}
        (
            data_reference_index,
            MediaData::Identified(_) | MediaData::SequenceNumber | MediaData::Elsewhere,
        ) => {
            return Err(Error::MediaDataElsewhere {
                track_id,
                data_reference_index,
            });
        }
    }
    location
        .offset
        .checked_add(u64::from(location.size))
        .ok_or(Error::Overflow { track_id })?;
    let sample_composition_time_offset = i32::try_from(properties.sample_composition_time_offset)
        .map_err(|_| Error::CompositionOffsetOutOfRange {
        sample_composition_time_offset: properties.sample_composition_time_offset,
    })?;
    Ok(Sample {
        offset: location.offset,
        size: location.size,
        sample_description_index,
        sample_duration: properties.sample_duration,
        sample_composition_time_offset,
        sample_flags: properties.sample_flags,
    })
}
