//! [`OpenFragment`], the samples of one movie fragment held until it is closed

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

use isobmff_boxes::{
    CompositionTimeOffset, MediaDataBox, MovieFragmentBox, MovieFragmentHeaderBox, SampleFlags,
    TrackBox, TrackExtendsBox, TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentBox,
    TrackFragmentHeaderBox, TrackFragmentHeaderFlags, TrackRunBox, TrackRunSample,
};
use isobmff_core::{BoxDefinition as _, BoxEncode as _};

use crate::error::Error;
use crate::sample::Sample;
use crate::sample_description::SampleDescriptions;
use crate::track_decode_times::TrackDecodeTimes;

/// Samples of one track lying next to each other in the media data of a fragment
///
/// `data_offset` is where the run starts in that media data, and every row
/// states all four fields of its sample.
#[derive(Clone, Debug)]
struct OpenRun {
    data_offset: u64,
    rows: Vec<TrackRunSample>,
}

/// Samples one track contributes to the fragment being written
///
/// `decode_time` is where the fragment places the track, which its `tfdt`
/// states, and `reached` where the samples added so far leave its media
/// timeline. `origin` is the decode time the first of those samples states,
/// which the samples after it are checked against in the times they state.
#[derive(Clone, Debug)]
struct OpenTrack {
    track_id: u32,
    origin: u64,
    decode_time: u64,
    reached: u64,
    sample_description_index: u32,
    runs: Vec<OpenRun>,
}

impl OpenTrack {
    /// Adds `row`, lasting `sample_duration`, to this track, in the run it carries on or one starting at `data_offset`
    ///
    /// `carries_on` states whether the sample handed over before this one
    /// belonged to this track, which is what makes the two lie next to each
    /// other in the media data.
    fn place(
        &mut self,
        row: TrackRunSample,
        sample_duration: u32,
        data_offset: u64,
        carries_on: bool,
    ) -> Result<(), Error> {
        self.reached = self
            .reached
            .checked_add(u64::from(sample_duration))
            .ok_or(Error::decode_time_overflow(self.track_id))?;

        match self.runs.last_mut() {
            Some(run) if carries_on => run.rows.push(row),
            _no_run_this_sample_carries_on => self.runs.push(OpenRun {
                data_offset,
                rows: vec![row],
            }),
        }

        Ok(())
    }

    /// Returns every row of every run of the track, in the order they were placed
    fn rows(&self) -> impl Iterator<Item = &TrackRunSample> {
        self.runs.iter().flat_map(|run| &run.rows)
    }
}

/// Fragment being written, holding its samples until it is closed
///
/// The tracks lie in the order they first appeared, which is the order their
/// `traf` boxes are written in, and `placed_tracks` names where each one lies.
/// `placement` is where the fragment places a track: at the decode time it
/// knows for the track, or where the first sample of a track it does not know
/// states.
#[derive(Clone, Debug)]
pub(super) struct OpenFragment {
    sequence_number: u32,
    placement: TrackDecodeTimes,
    tracks: Vec<OpenTrack>,
    placed_tracks: BTreeMap<u32, usize>,
    media_data: Vec<u8>,
    last_track_id: Option<u32>,
}

impl OpenFragment {
    /// Opens a fragment carrying no samples yet, numbered `sequence_number` by its `mfhd` and placing its tracks at `placement`
    pub(super) const fn new(sequence_number: u32, placement: TrackDecodeTimes) -> Self {
        Self {
            sequence_number,
            placement,
            tracks: Vec::new(),
            placed_tracks: BTreeMap::new(),
            media_data: Vec::new(),
            last_track_id: None,
        }
    }

    /// Places `sample` in the fragment, its bytes on the end of the media data
    ///
    /// A track reaching this fragment for the first time is checked against
    /// `trak` and `trex`, which the movie declares its tracks by, and against
    /// `decode_times`, where the fragments closed before this one leave each
    /// track.
    pub(super) fn place(
        &mut self,
        sample: Sample,
        trak: &[TrackBox],
        trex: &[TrackExtendsBox],
        decode_times: &TrackDecodeTimes,
    ) -> Result<(), Error> {
        let track_id = sample.track_id();
        let offered = sample.data().len() as u64;
        let Ok(sample_size) = u32::try_from(offered) else {
            return Err(Error::sample_size_out_of_range(track_id, offered));
        };
        let offset = sample.sample_composition_time_offset();
        let Some(sample_composition_time_offset) = CompositionTimeOffset::new(offset) else {
            return Err(Error::composition_time_offset_out_of_range(
                track_id, offset,
            ));
        };

        let sample_duration = sample.sample_duration();
        let row = TrackRunSample::new(
            Some(sample_duration),
            Some(sample_size),
            Some(sample.sample_flags()),
            Some(sample_composition_time_offset),
        );
        let decode_time = sample.decode_time();
        let sample_description_index = sample.sample_description_index();
        let data_offset = self.media_data.len() as u64;
        let carries_on = self.last_track_id == Some(track_id);

        // Why not scanning the tracks of the fragment per sample: a caller may
        // hand over a sample of every track a movie declares, so the scan would
        // cost the product of two figures the input settles.
        match self
            .placed_tracks
            .get(&track_id)
            .copied()
            .and_then(|position| self.tracks.get_mut(position))
        {
            Some(track) => {
                if track.sample_description_index != sample_description_index {
                    return Err(Error::sample_description_index_mismatch(
                        track_id,
                        sample_description_index,
                        track.sample_description_index,
                    ));
                }
                let expected = track
                    .origin
                    .checked_add(track.reached.saturating_sub(track.decode_time))
                    .ok_or(Error::decode_time_overflow(track_id))?;
                if expected != decode_time {
                    return Err(Error::decode_time_mismatch(track_id, decode_time, expected));
                }

                track.place(row, sample_duration, data_offset, carries_on)?;
            }
            None => {
                let trak = trak.iter().find(|trak| trak.tkhd().track_id() == track_id);
                let trex = trex.iter().find(|trex| trex.track_id() == track_id);
                let (Some(trak), Some(_trex)) = (trak, trex) else {
                    return Err(Error::unknown_track_id(track_id));
                };
                SampleDescriptions::new(trak).data_reference_index(sample_description_index)?;
                let placed = self.placement.decode_time(track_id).unwrap_or(decode_time);
                if let Some(reached) = decode_times
                    .decode_time(track_id)
                    .filter(|reached| placed < *reached)
                {
                    return Err(Error::backward_decode_time(track_id, placed, reached));
                }

                let mut track = OpenTrack {
                    track_id,
                    origin: decode_time,
                    decode_time: placed,
                    reached: placed,
                    sample_description_index,
                    runs: Vec::new(),
                };
                track.place(row, sample_duration, data_offset, false)?;
                self.placed_tracks.insert(track_id, self.tracks.len());
                self.tracks.push(track);
            }
        }

        self.media_data.extend_from_slice(sample.data());
        self.last_track_id = Some(track_id);

        Ok(())
    }

    /// Builds the `moof` and the `mdat` payload the fragment is written as, now that its samples are over
    ///
    /// Once both are built, `decode_times` is moved to where the samples of
    /// the fragment leave the timeline of each track it carries; a failure
    /// leaves it as it was.
    pub(super) fn into_boxes(
        self,
        decode_times: &mut TrackDecodeTimes,
    ) -> Result<(MovieFragmentBox, Vec<u8>), Error> {
        let measured = build_movie_fragment(self.sequence_number, &self.tracks, None)?;
        let media_data = MediaDataBox::new(self.media_data);
        let header_len = media_data
            .encoded_len()
            .saturating_sub(media_data.data().len() as u64);
        let base = measured.encoded_len().saturating_add(header_len);
        let movie_fragment = build_movie_fragment(self.sequence_number, &self.tracks, Some(base))?;
        for track in &self.tracks {
            decode_times.reach(track.track_id, track.reached);
        }

        Ok((movie_fragment, media_data.into_data()))
    }
}

/// What the samples of one track fragment share, and so what its `tfhd` states
///
/// A field every sample of the fragment states the same value for is written
/// once as the default of the `tfhd`, which
/// [`TrackRunBox::without_defaults`] then leaves out of the rows of every run.
/// A fragment whose samples share their flags but for the first one states the
/// shared ones, so that the flags of the first sample are written as its
/// `first_sample_flags` (ISO/IEC 14496-12 §8.8.8) against that default.
#[derive(Clone, Copy, Debug)]
struct Defaults {
    sample_duration: Option<u32>,
    sample_size: Option<u32>,
    sample_flags: Option<SampleFlags>,
}

impl Defaults {
    /// Returns what the samples of `track` share
    fn of(track: &OpenTrack) -> Self {
        let flags = || track.rows().map(TrackRunSample::sample_flags);

        Self {
            sample_duration: shared(track.rows().map(TrackRunSample::sample_duration)).flatten(),
            sample_size: shared(track.rows().map(TrackRunSample::sample_size)).flatten(),
            sample_flags: shared(flags())
                .or_else(|| shared(flags().skip(1)))
                .flatten(),
        }
    }
}

/// Returns the value every item of `values` is, when they are all one value
fn shared<Value: PartialEq, Values: Iterator<Item = Value>>(mut values: Values) -> Option<Value> {
    let first = values.next()?;

    values.all(|value| value == first).then_some(first)
}

/// Builds the `moof` the samples of one fragment are written as
///
/// `base` is where the media data of the fragment lies, counted from the start
/// of the `moof` — the anchor `default-base-is-moof` establishes (ISO/IEC
/// 14496-12 §8.8.7.1). `None` builds the same boxes with every offset zero,
/// which states the length of the `moof` that those offsets are counted from.
fn build_movie_fragment(
    sequence_number: u32,
    tracks: &[OpenTrack],
    base: Option<u64>,
) -> Result<MovieFragmentBox, Error> {
    // Why not measuring with a base of zero and dropping the `Option`: the
    // offsets are checked against the field that carries them as they are
    // built, and a fragment past that field would then be refused naming the
    // offset it holds in the media data rather than the one that did not fit.
    let track_fragments = tracks
        .iter()
        .map(|track| build_track_fragment(track, base))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(MovieFragmentBox::new(
        MovieFragmentHeaderBox::new(sequence_number),
        track_fragments,
    ))
}

/// Builds the `traf` the samples of one track of one fragment are written as
fn build_track_fragment(track: &OpenTrack, base: Option<u64>) -> Result<TrackFragmentBox, Error> {
    let defaults = Defaults::of(track);
    let header = TrackFragmentHeaderBox::new(
        TrackFragmentHeaderFlags::DEFAULT_BASE_IS_MOOF,
        track.track_id,
        None,
        Some(track.sample_description_index),
        defaults.sample_duration,
        defaults.sample_size,
        defaults.sample_flags,
    );

    let runs = track
        .runs
        .iter()
        .map(|run| {
            let data_offset = match base {
                Some(base) => {
                    let offset = base.saturating_add(run.data_offset);

                    i32::try_from(offset).map_err(|_past_the_field| {
                        Error::data_offset_out_of_range(track.track_id, offset)
                    })?
                }
                None => 0,
            };
            let track_run = TrackRunBox::new(Some(data_offset), None, run.rows.clone())
                .ok_or_else(|| {
                    let widest = run.rows.iter().filter_map(|row| {
                        row.sample_composition_time_offset()
                            .map(CompositionTimeOffset::get)
                    });
                    Error::composition_time_offset_out_of_range(
                        track.track_id,
                        widest.max().unwrap_or_default(),
                    )
                })?;

            Ok(track_run.without_defaults(&header))
        })
        .collect::<Result<Vec<_>, Error>>()?;

    let track_fragment = TrackFragmentBox::new(header, runs).ok_or(
        isobmff_core::Error::forbidden_child_box(TrackRunBox::BOX_TYPE),
    )?;

    Ok(track_fragment.with_tfdt(TrackFragmentBaseMediaDecodeTimeBox::new(track.decode_time)))
}

#[cfg(test)]
mod tests;
