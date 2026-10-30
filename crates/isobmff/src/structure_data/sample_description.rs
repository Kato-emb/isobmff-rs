//! The sample entry that describes samples, and the data reference entry that says where their media data is, ISO/IEC 14496-12 §8.5.2 and §8.7.2

use isobmff_boxes::{DataEntry, SampleDescription, TrackBox};

use crate::error::{DemuxError, MuxError};

/// Reason the samples a sample entry of a track describes are not described, or do not lie in the file itself
///
/// Reading and writing a file refuse the same samples, each with the kind of
/// the same name in its own error.
#[derive(Clone, Copy, Debug)]
pub(super) enum DataReferenceRefusal {
    /// The `stsd` has no entry `sample_description_index`, or the entry is too short to hold a `data_reference_index`
    UnknownSampleDescriptionIndex {
        track_id: u32,
        sample_description_index: u32,
    },
    /// The `dref` has no entry `data_reference_index`
    UnknownDataReferenceIndex {
        track_id: u32,
        data_reference_index: u16,
    },
    /// The entry names a file other than the file itself (§8.7.2)
    ExternalDataReference {
        track_id: u32,
        data_reference_index: u16,
    },
    /// The entry names an `IdentifiedMediaDataBox`
    UnsupportedDataReference {
        track_id: u32,
        data_reference_index: u16,
    },
}

impl From<DataReferenceRefusal> for DemuxError {
    fn from(refusal: DataReferenceRefusal) -> Self {
        match refusal {
            DataReferenceRefusal::UnknownSampleDescriptionIndex {
                track_id,
                sample_description_index,
            } => Self::UnknownSampleDescriptionIndex {
                track_id,
                sample_description_index,
            },
            DataReferenceRefusal::UnknownDataReferenceIndex {
                track_id,
                data_reference_index,
            } => Self::UnknownDataReferenceIndex {
                track_id,
                data_reference_index,
            },
            DataReferenceRefusal::ExternalDataReference {
                track_id,
                data_reference_index,
            } => Self::ExternalDataReference {
                track_id,
                data_reference_index,
            },
            DataReferenceRefusal::UnsupportedDataReference {
                track_id,
                data_reference_index,
            } => Self::UnsupportedDataReference {
                track_id,
                data_reference_index,
            },
        }
    }
}

impl From<DataReferenceRefusal> for MuxError {
    fn from(refusal: DataReferenceRefusal) -> Self {
        match refusal {
            DataReferenceRefusal::UnknownSampleDescriptionIndex {
                track_id,
                sample_description_index,
            } => Self::UnknownSampleDescriptionIndex {
                track_id,
                sample_description_index,
            },
            DataReferenceRefusal::UnknownDataReferenceIndex {
                track_id,
                data_reference_index,
            } => Self::UnknownDataReferenceIndex {
                track_id,
                data_reference_index,
            },
            DataReferenceRefusal::ExternalDataReference {
                track_id,
                data_reference_index,
            } => Self::ExternalDataReference {
                track_id,
                data_reference_index,
            },
            DataReferenceRefusal::UnsupportedDataReference {
                track_id,
                data_reference_index,
            } => Self::UnsupportedDataReference {
                track_id,
                data_reference_index,
            },
        }
    }
}

/// Returns the index of the entry of the `dref` of `trak` that names where the samples sample entry `sample_description_index` describes are, once it names the file itself
///
/// # Errors
///
/// * The [`DataReferenceRefusal`] of a sample entry the `stsd` has none of,
///   of a data reference entry the `dref` has none of, and of one that names
///   another file or an `IdentifiedMediaDataBox`.
pub(super) fn data_reference_index(
    trak: &TrackBox,
    sample_description_index: u32,
) -> Result<u16, DataReferenceRefusal> {
    let track_id = trak.tkhd.track_id;
    let unknown_description = DataReferenceRefusal::UnknownSampleDescriptionIndex {
        track_id,
        sample_description_index,
    };
    let description = nth_from_one(
        &trak.mdia.minf.stbl.stsd.sample_descriptions,
        sample_description_index,
    )
    .ok_or(unknown_description)?;
    let data_reference_index = match description {
        SampleDescription::Typed(entry) => entry.data_reference_index,
        // Why not refuse an entry that does not read as a `SampleEntry`: every sample entry opens with six reserved bytes and its `data_reference_index` (§8.5.2.2), whatever its format.
        SampleDescription::Other(entry) => entry
            .raw_payload()
            .and_then(|payload| payload.get(6..8))
            .and_then(|bytes| <[u8; 2]>::try_from(bytes).ok())
            .map(u16::from_be_bytes)
            .ok_or(unknown_description)?,
        _ => return Err(unknown_description),
    };

    match nth_from_one(
        &trak.mdia.minf.dinf.dref.data_entries,
        u32::from(data_reference_index),
    ) {
        Some(DataEntry::Url(url)) if url.location.is_none() => Ok(data_reference_index),
        Some(DataEntry::Imda(_) | DataEntry::SeqNumImda(_)) => {
            Err(DataReferenceRefusal::UnsupportedDataReference {
                track_id,
                data_reference_index,
            })
        }
        Some(_) => Err(DataReferenceRefusal::ExternalDataReference {
            track_id,
            data_reference_index,
        }),
        None => Err(DataReferenceRefusal::UnknownDataReferenceIndex {
            track_id,
            data_reference_index,
        }),
    }
}

/// Returns entry `index` of a table that numbers its entries from 1
fn nth_from_one<Entry>(entries: &[Entry], index: u32) -> Option<&Entry> {
    entries.get(usize::try_from(index.checked_sub(1)?).ok()?)
}
