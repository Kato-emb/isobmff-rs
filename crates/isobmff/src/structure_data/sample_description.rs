//! The sample entry that describes samples, and the data reference entry that says where their media data is, ISO/IEC 14496-12 §8.5.2 and §8.7.2

use isobmff_boxes::{DataEntry, MovieBox, SampleDescription, TrackBox};

use crate::error::{DemuxError, MuxError};
use crate::sample::DataReferenceKey;

/// One entry of the data reference of a track of the movie, under the key the locations of its samples carry, ISO/IEC 14496-12 §8.7.2
///
/// A [`StructureDataReader`](super::StructureDataReader) lists one for each
/// entry of the `dref` of each track: two tracks whose entries name the same
/// file are two of them. It borrows the entry from the movie the reader
/// holds.
///
/// # Examples
///
/// ```
/// use isobmff::boxes::{DataEntry, MovieBox, TrackBox};
/// use isobmff::structure_data::{StructureDataLimits, StructureDataReader};
///
/// let mut trak = TrackBox::default();
/// trak.tkhd.track_id = 1;
/// trak.mdia.minf.dinf.dref.data_entries = vec![DataEntry::Url(Default::default())];
/// let mut movie = MovieBox::default();
/// movie.trak = vec![trak];
/// let mut reader = StructureDataReader::new(StructureDataLimits {
///     sample_count: 1_048_576,
///     segment_index_count: 1024,
///     subsegment_count: 1_048_576,
///     leniency_count: 65_536,
/// });
/// reader.handle_movie(movie)?;
///
/// let data_reference = reader.data_references().next().ok_or("no entry")?;
/// assert_eq!((data_reference.track_id(), data_reference.index()), (1, 1));
/// assert!(data_reference.entry().is_in_same_file());
/// assert_eq!(reader.data_reference(data_reference.key()), Some(data_reference));
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DataReference<'a> {
    pub(super) key: DataReferenceKey,
    pub(super) track_id: u32,
    pub(super) index: u16,
    pub(super) entry: &'a DataEntry,
}

impl<'a> DataReference<'a> {
    /// Returns the key the locations of the samples whose sample entry names the entry carry
    #[must_use]
    pub const fn key(&self) -> DataReferenceKey {
        self.key
    }

    /// Returns the `track_ID` of the track whose `dref` holds the entry
    #[must_use]
    pub const fn track_id(&self) -> u32 {
        self.track_id
    }

    /// Returns the index, from 1, of the entry in the `dref`, as a sample entry names it in its `data_reference_index`
    #[must_use]
    pub const fn index(&self) -> u16 {
        self.index
    }

    /// Returns the entry, which says where the media data is
    #[must_use]
    pub const fn entry(&self) -> &'a DataEntry {
        self.entry
    }
}

/// Returns an entry of the data reference of each track of `movie` for each entry its `dref` has, in the order of their keys
///
/// The entries past the last one a `data_reference_index` can number are
/// left out: no sample entry can name them.
pub(super) fn data_references(movie: &MovieBox) -> impl Iterator<Item = DataReference<'_>> {
    movie
        .trak
        .iter()
        .enumerate()
        .flat_map(|(track_position, trak)| {
            (1..=u16::MAX)
                .zip(&trak.mdia.minf.dinf.dref.data_entries)
                .map(move |(index, entry)| DataReference {
                    key: DataReferenceKey::of(track_position, index),
                    track_id: trak.tkhd.track_id,
                    index,
                    entry,
                })
        })
}

/// Returns the entry of the data reference of `movie` under key `key`, or `None` where no track and entry of `movie` has the key
pub(super) fn data_reference(movie: &MovieBox, key: DataReferenceKey) -> Option<DataReference<'_>> {
    let (track_position, index) = key.unpacked();
    let trak = movie.trak.get(usize::try_from(track_position).ok()?)?;
    let entry = nth_from_one(&trak.mdia.minf.dinf.dref.data_entries, u32::from(index))?;
    Some(DataReference {
        key,
        track_id: trak.tkhd.track_id,
        index,
        entry,
    })
}

/// Reason the samples a sample entry of a track describes are not described, or do not lie in the file itself
///
/// Writing a file refuses these samples, each with the kind of the same name
/// in [`MuxError`]. Reading a file refuses the same ones but those whose
/// media data is in another file, which it locates in that file.
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

    let data_entries = &trak.mdia.minf.dinf.dref.data_entries;
    match nth_from_one(data_entries, u32::from(data_reference_index)) {
        Some(entry) if entry.is_in_same_file() => Ok(data_reference_index),
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

/// Returns the key of the entry of the `dref` of `trak`, the track at `track_position` of the movie, that names where the samples sample entry `sample_description_index` describes are, in the file itself or in another
///
/// # Errors
///
/// * [`UnknownSampleDescriptionIndex`](DemuxError::UnknownSampleDescriptionIndex),
///   [`UnknownDataReferenceIndex`](DemuxError::UnknownDataReferenceIndex):
///   the `stsd` has no such sample entry, or the `dref` no entry the sample
///   entry names.
/// * [`UnsupportedDataReference`](DemuxError::UnsupportedDataReference): the
///   entry names an `IdentifiedMediaDataBox`.
pub(super) fn located_data_reference(
    trak: &TrackBox,
    track_position: usize,
    sample_description_index: u32,
) -> Result<DataReferenceKey, DemuxError> {
    match data_reference_index(trak, sample_description_index) {
        Ok(data_reference_index)
        | Err(DataReferenceRefusal::ExternalDataReference {
            data_reference_index,
            ..
        }) => Ok(DataReferenceKey::of(track_position, data_reference_index)),
        Err(DataReferenceRefusal::UnknownSampleDescriptionIndex {
            track_id,
            sample_description_index,
        }) => Err(DemuxError::UnknownSampleDescriptionIndex {
            track_id,
            sample_description_index,
        }),
        Err(DataReferenceRefusal::UnknownDataReferenceIndex {
            track_id,
            data_reference_index,
        }) => Err(DemuxError::UnknownDataReferenceIndex {
            track_id,
            data_reference_index,
        }),
        Err(DataReferenceRefusal::UnsupportedDataReference {
            track_id,
            data_reference_index,
        }) => Err(DemuxError::UnsupportedDataReference {
            track_id,
            data_reference_index,
        }),
    }
}

/// Returns entry `index` of a table that numbers its entries from 1
fn nth_from_one<Entry>(entries: &[Entry], index: u32) -> Option<&Entry> {
    entries.get(usize::try_from(index.checked_sub(1)?).ok()?)
}
