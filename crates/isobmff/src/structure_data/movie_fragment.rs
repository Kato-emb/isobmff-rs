//! The samples a movie fragment declares, and where their bytes lie

use alloc::vec::Vec;

use isobmff_boxes::{
    DataReferenceBox, MovieFragmentBox, SampleDescriptionBox, TrackExtendsBox, TrackRunBox,
    TrackRunSample, TrackRunSamples,
};

use isobmff_core::BoxDefinition;

use super::Error;
use super::identified_media_data::{IdentifiedMediaData, Origin};
use super::reader::Reading;
use super::sample::{self, MediaData};
use crate::{SampleLocation, SampleProperties};

/// What a track of the `moov` gives the samples of its track fragments
#[derive(Debug, Clone)]
pub(super) struct FragmentTrack {
    track_id: u32,
    /// The defaults of the `trex` of the track, §8.8.3
    defaults: Option<TrackExtendsBox>,
    /// The `data_reference_index` of each sample entry of the track, from 1,
    /// and where it places the media data
    entries: Vec<Option<(u16, MediaData)>>,
    /// Decoding time of the next sample of the track: the sum of the
    /// `sample_duration`s of the samples before it, §8.8.12, or the `tfdt`
    /// of the last track fragment that gave one; `None` before any
    next_decoding_time: Option<u64>,
}

impl FragmentTrack {
    /// Track `track_id`, whose samples in the `moov` end at `next_decoding_time`, or `None` for no sample
    ///
    /// `defaults` are those of the `trex` of the track, and `stsd` and `dref`
    /// its sample entries and data references.
    pub(super) fn new(
        track_id: u32,
        defaults: Option<TrackExtendsBox>,
        stsd: &SampleDescriptionBox,
        dref: &DataReferenceBox,
        next_decoding_time: Option<u64>,
    ) -> Self {
        Self {
            track_id,
            defaults,
            entries: sample::entry_data_references(stsd, dref),
            next_decoding_time,
        }
    }
    /// `track_ID` of the track
    pub(super) const fn track_id(&self) -> u32 {
        self.track_id
    }

    /// Whether the samples of the sample entry `sample_description_index`, from 1, lie in the file of the `dref`, or in an `imda` of it
    ///
    /// # Errors
    /// * [`Error::SampleDescriptionIndexOutOfRange`]: the track has no such
    ///   sample entry.
    /// * [`Error::InvalidEntry`]: the sample entry ends before its
    ///   `data_reference_index`.
    /// * [`Error::DataReferenceIndexOutOfRange`]: the sample entry names no
    ///   data reference entry.
    pub(super) fn is_in_this_file(&self, sample_description_index: u32) -> Result<bool, Error> {
        let track_id = self.track_id;
        match usize::try_from(sample_description_index)
            .ok()
            .and_then(|index| index.checked_sub(1))
            .and_then(|index| self.entries.get(index))
        {
            Some(&Some((
                _,
                MediaData::SameFile | MediaData::Identified(_) | MediaData::SequenceNumber,
            ))) => Ok(true),
            Some(&Some((_, MediaData::Elsewhere))) => Ok(false),
            Some(&Some((data_reference_index, MediaData::NoEntry))) => {
                Err(Error::DataReferenceIndexOutOfRange {
                    track_id,
                    data_reference_index,
                })
            }
            Some(None) => Err(Error::InvalidEntry {
                track_id,
                box_type: SampleDescriptionBox::BOX_TYPE,
                entry: u64::from(sample_description_index.saturating_sub(1)),
            }),
            None => Err(Error::SampleDescriptionIndexOutOfRange {
                sample_description_index,
            }),
        }
    }
}

/// Returns the number of samples the track runs of `moof` declare
///
/// The `sample_count` of each `trun` of each `traf`, ISO/IEC 14496-12
/// §8.8.8, saturating at `u64::MAX`. A caller that holds the samples of
/// several boxes counts those of a `moof` with it before it hands the box to
/// [`StructureDataReader::handle_movie_fragment`](super::StructureDataReader::handle_movie_fragment).
#[must_use]
pub fn movie_fragment_sample_count(moof: &MovieFragmentBox) -> u64 {
    moof.traf
        .iter()
        .flat_map(|traf| &traf.trun)
        .map(|trun| u64::try_from(run_len(trun)).unwrap_or(u64::MAX))
        .fold(0, u64::saturating_add)
}

/// Where a sample lies among the boxes of a `moof`: the `traf` that holds it, the `trun` of that `traf`, and its place in the `trun`
///
/// The numbers of an entry of a
/// [`TrackFragmentRandomAccessBox`](isobmff_boxes::TrackFragmentRandomAccessBox),
/// ISO/IEC 14496-12 §8.8.10, each from 1.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SamplePosition {
    /// Number of the `traf` that holds the sample, among those of the `moof`
    pub traf_number: u32,
    /// Number of the `trun` that holds the sample, among those of its `traf`
    pub trun_number: u32,
    /// Number of the sample among those of its `trun`
    pub sample_delta: u32,
}

/// Returns where the sample at `index`, from 0, of the samples of track `track_id` in `moof`, in decoding order, lies among its boxes, or `None` where the track has fewer samples there
///
/// The samples of a track in a `moof` are those of its `traf`s, in their
/// order, and of their `trun`s, in theirs, ISO/IEC 14496-12 §8.8.6 and
/// §8.8.8; their positions are what a `tfra` entry names, §8.8.10.
///
/// ```
/// use isobmff::boxes::{MovieFragmentBox, TrackFragmentBox, TrackRunBox, TrackRunSamples};
/// use isobmff::structure_data::sample_position;
///
/// // A `traf` of track 1 of runs of 2 and 3 samples, after one of track 2
/// let traf = |track_id, runs: &[u32]| {
///     let mut traf = TrackFragmentBox::default();
///     traf.tfhd.track_id = track_id;
///     traf.trun = runs
///         .iter()
///         .map(|&sample_count| {
///             let mut trun = TrackRunBox::default();
///             trun.samples = TrackRunSamples::EmptyRows { sample_count };
///             trun
///         })
///         .collect();
///     traf
/// };
/// let mut moof = MovieFragmentBox::default();
/// moof.traf = vec![traf(2, &[1]), traf(1, &[2, 3])];
///
/// // The fourth sample of track 1 is the second of its second `trun`
/// let position = sample_position(&moof, 1, 3).ok_or("no sample")?;
/// assert_eq!((position.traf_number, position.trun_number, position.sample_delta), (2, 2, 2));
/// assert_eq!(sample_position(&moof, 1, 5), None);
/// # Ok::<(), &str>(())
/// ```
#[must_use]
pub fn sample_position(
    moof: &MovieFragmentBox,
    track_id: u32,
    index: u64,
) -> Option<SamplePosition> {
    let mut before = index;
    for (traf_index, traf) in moof.traf.iter().enumerate() {
        if traf.tfhd.track_id != track_id {
            continue;
        }
        for (trun_index, trun) in traf.trun.iter().enumerate() {
            let sample_count = u64::try_from(run_len(trun)).unwrap_or(u64::MAX);
            if before < sample_count {
                return Some(SamplePosition {
                    traf_number: u32::try_from(traf_index.checked_add(1)?).ok()?,
                    trun_number: u32::try_from(trun_index.checked_add(1)?).ok()?,
                    sample_delta: u32::try_from(before.checked_add(1)?).ok()?,
                });
            }
            before = before.saturating_sub(sample_count);
        }
    }
    None
}

/// Appends the samples of `moof`, the `moof` at file offset `moof_offset`, to `locations`, or to the samples of `identified` that wait for an `imda`, and moves the decoding time of each track on past them
///
/// A `tfdt` past the decoding time the samples before it in its track add
/// up to is taken, §8.8.12.1. One before it is refused as the specification
/// reads it, and taken when read lossily. Leaves `tracks` as they were
/// where it refuses the box.
pub(super) fn resolve(
    moof: &MovieFragmentBox,
    moof_offset: u64,
    tracks: &mut [FragmentTrack],
    locations: &mut Vec<SampleLocation>,
    identified: &mut IdentifiedMediaData,
    reading: Reading,
) -> Result<(), Error> {
    // Why move the decoding times on in a copy: a box refused at a later
    // track fragment leaves the tracks as they were.
    let mut decoding_times: Vec<Option<u64>> = tracks
        .iter()
        .map(|track| track.next_decoding_time)
        .collect();
    // Why carry the end of the data of the preceding track fragment: a
    // `tfhd` that gives no base and is not of the first track fragment
    // counts from there, Table 6 of §8.8.7.1, in the coordinates that data
    // lies in.
    let mut previous_end: Option<(Origin, u64)> = None;
    for traf in &moof.traf {
        let tfhd = &traf.tfhd;
        let track_id = tfhd.track_id;
        let overflow = Error::Overflow { track_id };
        let missing = Error::MissingDefault { track_id };
        let (track, decoding_time) = tracks
            .iter()
            .zip(decoding_times.iter_mut())
            .find(|(track, _)| track.track_id == track_id)
            .ok_or(Error::UnknownTrack { track_id })?;
        let trex = track.defaults.as_ref();
        let default_duration = tfhd
            .default_sample_duration
            .or(trex.map(|trex| trex.default_sample_duration));
        let default_size = tfhd
            .default_sample_size
            .or(trex.map(|trex| trex.default_sample_size));
        let default_flags = tfhd
            .default_sample_flags
            .or(trex.map(|trex| trex.default_sample_flags));
        let has_samples = traf.trun.iter().any(|trun| run_len(trun) > 0);
        let sample_description_index = tfhd
            .sample_description_index
            .or(trex.map(|trex| trex.default_sample_description_index));
        // Why the identified media data only of a track fragment with
        // samples: no sample entry is named by one without, and its data
        // ends where it starts, so no origin is needed.
        let identified_media_data = if has_samples {
            let sample_description_index = sample_description_index.ok_or(missing)?;
            let entry = usize::try_from(sample_description_index)
                .ok()
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| track.entries.get(index));
            match entry {
                // Why a file offset for a sample in another file: Table 6
                // counts its data from the base the `tfhd` gives in that file,
                // or from where it counts that of any other.
                Some(&Some((_, MediaData::SameFile | MediaData::Elsewhere))) => None,
                Some(&Some((_, MediaData::Identified(imda_identifier)))) => Some(imda_identifier),
                Some(&Some((_, MediaData::SequenceNumber))) => Some(moof.mfhd.sequence_number),
                Some(&Some((data_reference_index, MediaData::NoEntry))) => {
                    return Err(Error::DataReferenceIndexOutOfRange {
                        track_id,
                        data_reference_index,
                    });
                }
                Some(None) => {
                    return Err(Error::InvalidEntry {
                        track_id,
                        box_type: SampleDescriptionBox::BOX_TYPE,
                        entry: u64::from(sample_description_index.saturating_sub(1)),
                    });
                }
                None => {
                    return Err(Error::SampleDescriptionIndexOutOfRange {
                        sample_description_index,
                    });
                }
            }
        } else {
            None
        };
        let (origin, base) = match (
            tfhd.base_data_offset,
            tfhd.default_base_is_moof,
            previous_end,
            identified_media_data,
        ) {
            (Some(base), _, _, Some(imda_identifier)) => {
                (Origin::IdentifiedMediaData(imda_identifier), base)
            }
            (Some(base), _, _, None) => (Origin::File, base),
            (None, false, Some(end), _) => end,
            (None, false, None, Some(imda_identifier)) => {
                (Origin::IdentifiedMediaData(imda_identifier), 0)
            }
            (None, true, _, _) | (None, false, None, None) => (Origin::File, moof_offset),
        };
        if let Some(tfdt) = &traf.tfdt {
            let found = tfdt.base_media_decode_time;
            if let Some(expected) = *decoding_time
                && found < expected
                && reading == Reading::Specification
            {
                return Err(Error::DecodingTimeMismatch {
                    track_id,
                    expected,
                    found,
                });
            }
            *decoding_time = Some(found);
        }
        let time = decoding_time.get_or_insert(0);
        let mut end = base;
        for trun in &traf.trun {
            let mut offset = match trun.data_offset {
                Some(data_offset) => base
                    .checked_add_signed(i64::from(data_offset))
                    .ok_or(overflow)?,
                None => end,
            };
            for (number, row) in rows(trun).enumerate() {
                let sample_duration = row.sample_duration.or(default_duration).ok_or(missing)?;
                let size = row.sample_size.or(default_size).ok_or(missing)?;
                let sample_flags = trun
                    .first_sample_flags
                    .filter(|_| number == 0)
                    .or(row.sample_flags)
                    .or(default_flags)
                    .ok_or(missing)?;
                identified.place(
                    origin,
                    identified_media_data,
                    SampleLocation {
                        properties: SampleProperties {
                            track_id,
                            decoding_time: *time,
                            sample_duration,
                            sample_composition_time_offset: row
                                .sample_composition_time_offset
                                .unwrap_or(0),
                            sample_flags,
                            sample_description_index: sample_description_index.unwrap_or(0),
                        },
                        offset,
                        size,
                    },
                    locations,
                )?;
                *time = time
                    .checked_add(u64::from(sample_duration))
                    .ok_or(overflow)?;
                offset = offset.checked_add(u64::from(size)).ok_or(overflow)?;
            }
            end = offset;
        }
        previous_end = Some((origin, end));
    }
    for (track, decoding_time) in tracks.iter_mut().zip(decoding_times) {
        track.next_decoding_time = decoding_time;
    }
    Ok(())
}

/// Number of samples `trun` adds
fn run_len(trun: &TrackRunBox) -> usize {
    match &trun.samples {
        TrackRunSamples::EmptyRows { sample_count } => {
            usize::try_from(*sample_count).unwrap_or(usize::MAX)
        }
        TrackRunSamples::Rows { samples } => samples.len(),
        _ => 0,
    }
}

/// The rows of `trun`, each sample of a run of empty rows as a row that carries no field
fn rows(trun: &TrackRunBox) -> impl Iterator<Item = TrackRunSample> + '_ {
    let (rows, empty_rows) = match &trun.samples {
        TrackRunSamples::Rows { samples } => (samples.as_slice(), 0),
        TrackRunSamples::EmptyRows { .. } => (&[][..], run_len(trun)),
        _ => (&[][..], 0),
    };
    rows.iter()
        .copied()
        .chain(core::iter::repeat_n(TrackRunSample::default(), empty_rows))
}
