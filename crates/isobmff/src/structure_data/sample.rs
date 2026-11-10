//! [`TrackSampleDescription`], and what the writers of structure-data hold of a sample and check of it on input

use alloc::vec::Vec;

use isobmff_boxes::{DataReferenceBox, SampleDescriptionBox, SampleFlags};

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

/// The `data_reference_index` of each sample entry of `stsd`, from 1, and whether the entry of `dref` it names places the media data in the same file
pub(super) fn entry_data_references(
    stsd: &SampleDescriptionBox,
    dref: &DataReferenceBox,
) -> Vec<(u16, bool)> {
    stsd.sample_descriptions
        .iter()
        .map(|entry| {
            let data_reference_index = entry.data_reference_index().unwrap_or(0);
            (
                data_reference_index,
                is_in_same_file(dref, data_reference_index),
            )
        })
        .collect()
}

/// Whether the data reference entry `data_reference_index` names, from 1, places the media data in the same file
pub(super) fn is_in_same_file(dref: &DataReferenceBox, data_reference_index: u16) -> bool {
    usize::from(data_reference_index)
        .checked_sub(1)
        .and_then(|index| dref.data_entries.get(index))
        .is_some_and(isobmff_boxes::DataEntry::is_in_same_file)
}

/// Returns what a writer holds of the sample at `location`, which joins `held` samples under `sample_count_limit`, its sample entry one of `entries`
///
/// # Errors
/// * [`Error::SampleCountOverLimit`]: the writer holds as many samples as
///   the limit, or as a table or a track fragment run counts.
/// * [`Error::SampleDescriptionIndexOutOfRange`]: the sample names no entry
///   of `entries`.
/// * [`Error::MediaDataElsewhere`]: the entry places the media data in
///   another file, or under no data reference entry.
/// * [`Error::Overflow`]: the bytes of the sample end past the last offset
///   there can be.
/// * [`Error::CompositionOffsetOutOfRange`]: the composition offset of the
///   sample does not fit 32 bits, signed.
pub(super) fn checked(
    location: &SampleLocation,
    entries: &[(u16, bool)],
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
    match usize::try_from(sample_description_index)
        .ok()
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| entries.get(index))
    {
        Some(&(_, true)) => {}
        Some(&(data_reference_index, false)) => {
            return Err(Error::MediaDataElsewhere {
                track_id,
                data_reference_index,
            });
        }
        None => {
            return Err(Error::SampleDescriptionIndexOutOfRange {
                sample_description_index,
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
