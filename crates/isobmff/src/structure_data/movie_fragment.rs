//! The samples a movie fragment declares, and where their bytes lie

use alloc::vec::Vec;

use isobmff_boxes::{
    MovieBox, MovieFragmentBox, TrackBox, TrackExtendsBox, TrackRunBox, TrackRunSample,
    TrackRunSamples,
};

use super::{Error, sample_table};
use crate::{SampleLocation, SampleProperties};

/// What a track of the `moov` gives the samples of its track fragments
#[derive(Debug, Clone)]
pub(super) struct FragmentTrack {
    track_id: u32,
    /// The defaults of the `trex` of the track, §8.8.3
    defaults: Option<TrackExtendsBox>,
    /// The `data_reference_index` of each sample entry of the track, from 1,
    /// and whether it places the media data in the same file
    entries: Vec<(u16, bool)>,
    /// Decoding time of the next sample of the track: the sum of the
    /// `sample_duration`s of the samples before it, §8.8.12
    next_decoding_time: u64,
}

impl FragmentTrack {
    /// The track `track` of `movie`, whose samples in the `moov` end at `next_decoding_time`
    pub(super) fn new(movie: &MovieBox, track: &TrackBox, next_decoding_time: u64) -> Self {
        let track_id = track.tkhd.track_id;
        Self {
            track_id,
            defaults: movie.mvex.as_ref().and_then(|mvex| {
                mvex.trex
                    .iter()
                    .find(|trex| trex.track_id == track_id)
                    .cloned()
            }),
            entries: sample_table::entry_data_references(track),
            next_decoding_time,
        }
    }
}

/// Number of samples the track runs of `moof` declare
pub(super) fn sample_count(moof: &MovieFragmentBox) -> u64 {
    moof.traf
        .iter()
        .flat_map(|traf| &traf.trun)
        .map(|trun| u64::try_from(run_len(trun)).unwrap_or(u64::MAX))
        .fold(0, u64::saturating_add)
}

/// Appends the samples of `moof`, the `moof` at file offset `moof_offset`, to `locations`, and moves the decoding time of each track on past them
///
/// Leaves `tracks` as they were where it refuses the box.
pub(super) fn resolve(
    moof: &MovieFragmentBox,
    moof_offset: u64,
    tracks: &mut [FragmentTrack],
    locations: &mut Vec<SampleLocation>,
) -> Result<(), Error> {
    // Why move the decoding times on in a copy: a box refused at a later
    // track fragment leaves the tracks as they were.
    let mut decoding_times: Vec<u64> = tracks
        .iter()
        .map(|track| track.next_decoding_time)
        .collect();
    // Why carry the end of the data of the preceding track fragment: a
    // `tfhd` that gives no base and is not of the first track fragment
    // counts from there, Table 6 of §8.8.7.1.
    let mut previous_end: Option<u64> = None;
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
            .or(trex.map(|trex| trex.default_sample_flags))
            .unwrap_or_default();
        let has_samples = traf.trun.iter().any(|trun| run_len(trun) > 0);
        let sample_description_index = tfhd
            .sample_description_index
            .or(trex.map(|trex| trex.default_sample_description_index));
        if has_samples {
            let sample_description_index = sample_description_index.ok_or(missing)?;
            let entry = usize::try_from(sample_description_index)
                .ok()
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| track.entries.get(index));
            match entry {
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
        }
        let base = match (
            tfhd.base_data_offset,
            tfhd.default_base_is_moof,
            previous_end,
        ) {
            (Some(base), _, _) => base,
            (None, false, Some(end)) => end,
            (None, true, _) | (None, false, None) => moof_offset,
        };
        if let Some(tfdt) = &traf.tfdt {
            *decoding_time = tfdt.base_media_decode_time;
        }
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
                    .unwrap_or(default_flags);
                locations.push(SampleLocation {
                    properties: SampleProperties {
                        track_id,
                        decoding_time: *decoding_time,
                        sample_duration,
                        sample_composition_time_offset: row
                            .sample_composition_time_offset
                            .unwrap_or(0),
                        sample_flags,
                        sample_description_index: sample_description_index.unwrap_or(0),
                    },
                    offset,
                    size,
                });
                *decoding_time = decoding_time
                    .checked_add(u64::from(sample_duration))
                    .ok_or(overflow)?;
                offset = offset.checked_add(u64::from(size)).ok_or(overflow)?;
            }
            end = offset;
        }
        previous_end = Some(end);
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
