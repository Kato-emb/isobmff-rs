//! The sample entry that describes samples, and the data reference entry that says where their media data is, ISO/IEC 14496-12 §8.5.2 and §8.7.2

use alloc::vec::Vec;

use isobmff_boxes::{DataEntry, MovieBox, SampleDescription, TrackBox};

use crate::byte_range::to_u64;
use crate::error::{DemuxError, MuxError};
use crate::sample::{DataReferenceKey, Sample, SampleLocation};

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
///     identified_media_data_count: 65_536,
///     unresolved_sample_count: 1_048_576,
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
pub(crate) enum DataReferenceRefusal {
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
pub(crate) fn data_reference_index(
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

/// Where the data offsets of the samples a data reference entry names count from, ISO/IEC 14496-12 §6.1 Table 1
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum DataOrigin {
    /// The start of the file the entry names, the file itself or another
    File,
    /// The first byte of the payload of the `IdentifiedMediaDataBox` of this `imda_identifier`, which a `DataEntryImdaBox` names
    IdentifiedMediaData(u32),
    /// The first byte of the payload of the `IdentifiedMediaDataBox` whose `imda_identifier` is the `sequence_number` of the movie fragment of the samples, as a `DataEntrySeqNumImdaBox` says
    SequenceNumber,
}

/// Returns the key of the entry of the `dref` of `trak`, the track at `track_position` of the movie, that names where the samples sample entry `sample_description_index` describes are, and where their data offsets count from
///
/// # Errors
///
/// * [`UnknownSampleDescriptionIndex`](DemuxError::UnknownSampleDescriptionIndex),
///   [`UnknownDataReferenceIndex`](DemuxError::UnknownDataReferenceIndex):
///   the `stsd` has no such sample entry, or the `dref` no entry the sample
///   entry names.
pub(super) fn located_data_reference(
    trak: &TrackBox,
    track_position: usize,
    sample_description_index: u32,
) -> Result<(DataReferenceKey, DataOrigin), DemuxError> {
    let track_id = trak.tkhd.track_id;
    let data_reference_index = match data_reference_index(trak, sample_description_index) {
        Ok(data_reference_index)
        | Err(
            DataReferenceRefusal::ExternalDataReference {
                data_reference_index,
                ..
            }
            | DataReferenceRefusal::UnsupportedDataReference {
                data_reference_index,
                ..
            },
        ) => data_reference_index,
        Err(DataReferenceRefusal::UnknownSampleDescriptionIndex {
            sample_description_index,
            ..
        }) => {
            return Err(DemuxError::UnknownSampleDescriptionIndex {
                track_id,
                sample_description_index,
            });
        }
        Err(DataReferenceRefusal::UnknownDataReferenceIndex {
            data_reference_index,
            ..
        }) => {
            return Err(DemuxError::UnknownDataReferenceIndex {
                track_id,
                data_reference_index,
            });
        }
    };
    let entries = &trak.mdia.minf.dinf.dref.data_entries;
    let origin = match nth_from_one(entries, u32::from(data_reference_index)) {
        Some(DataEntry::Imda(imdt)) => DataOrigin::IdentifiedMediaData(imdt.imda_ref_identifier),
        Some(DataEntry::SeqNumImda(_)) => DataOrigin::SequenceNumber,
        _ => DataOrigin::File,
    };

    Ok((
        DataReferenceKey::of(track_position, data_reference_index),
        origin,
    ))
}

/// Returns entry `index` of a table that numbers its entries from 1
fn nth_from_one<Entry>(entries: &[Entry], index: u32) -> Option<&Entry> {
    entries.get(usize::try_from(index.checked_sub(1)?).ok()?)
}

/// Returns the location of each of `samples` of the tracks of `movie`, their media data one after another in their order from file offset `data_offset`
///
/// Each location carries the key a
/// [`StructureDataReader`](super::StructureDataReader) gives the data
/// reference entry the sample entry of its sample names (§8.7.2).
///
/// # Errors
///
/// * [`UnknownTrackId`](MuxError::UnknownTrackId): a sample is of a track
///   the movie has none of.
/// * [`UnknownSampleDescriptionIndex`](MuxError::UnknownSampleDescriptionIndex),
///   [`UnknownDataReferenceIndex`](MuxError::UnknownDataReferenceIndex),
///   [`ExternalDataReference`](MuxError::ExternalDataReference),
///   [`UnsupportedDataReference`](MuxError::UnsupportedDataReference): the
///   track has no sample entry the sample names, or its entry names a data
///   reference entry that is not the file itself.
/// * [`DataOffsetOutOfRange`](MuxError::DataOffsetOutOfRange): the media
///   data of a sample lies past 64-bit file offsets.
pub(crate) fn laid_out(
    movie: &MovieBox,
    samples: &[Sample],
    data_offset: u64,
) -> Result<Vec<SampleLocation>, MuxError> {
    let mut data_end = data_offset;
    let mut described: Vec<(u32, u32, DataReferenceKey)> = Vec::new();
    samples
        .iter()
        .map(|sample| {
            let properties = sample.properties();
            let (track_id, sample_description_index) =
                (properties.track_id, properties.sample_description_index);
            let known = described
                .iter()
                .find(|&&(described_track, described_by, _)| {
                    (described_track, described_by) == (track_id, sample_description_index)
                })
                .map(|&(_, _, key)| key);
            let data_reference = match known {
                Some(key) => key,
                None => {
                    let (track_position, trak) = movie
                        .trak
                        .iter()
                        .enumerate()
                        .find(|(_, trak)| trak.tkhd.track_id == track_id)
                        .ok_or(MuxError::UnknownTrackId { track_id })?;
                    let key = DataReferenceKey::of(
                        track_position,
                        data_reference_index(trak, sample_description_index)?,
                    );
                    described.push((track_id, sample_description_index, key));
                    key
                }
            };
            let start = data_end;
            data_end = start
                .checked_add(to_u64(sample.data().len()))
                .ok_or(MuxError::DataOffsetOutOfRange { track_id })?;
            Ok(SampleLocation::new(
                *properties,
                data_reference,
                start..data_end,
            ))
        })
        .collect()
}
