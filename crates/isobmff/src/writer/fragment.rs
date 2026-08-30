//! [`OpenFragment`], the samples of one movie fragment held until it is closed

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use isobmff_boxes::{
    MediaDataBox, MovieFragmentBox, MovieFragmentHeaderBox, TrackFragmentBaseMediaDecodeTimeBox,
    TrackFragmentBox, TrackFragmentHeaderBox, TrackRunBox, TrackRunSample,
};
use isobmff_core::BoxEncode as _;

use crate::error::SampleError;
use crate::sample::Sample;

/// What one sample of a fragment states, beside the bytes it is carried as
#[derive(Clone, Copy, Debug)]
struct PlacedSample {
    sample_duration: u32,
    sample_size: u32,
    sample_flags: u32,
    sample_composition_time_offset: Option<i64>,
}

/// Samples of one track lying next to each other in the media data of a fragment
///
/// `data_offset` is where the run starts in that media data. A run states which
/// fields its rows carry once, for all of them, so a sample is added to the run
/// before it only where the two agree on those fields — see
/// [`takes`](Self::takes).
#[derive(Clone, Debug)]
struct OpenRun {
    data_offset: u64,
    samples: Vec<PlacedSample>,
    carries_offsets: bool,
    holds_negative_offset: bool,
    holds_wide_offset: bool,
}

impl OpenRun {
    /// Returns whether this run still writes with a sample stating `offset` added to it
    ///
    /// A row carries a composition time offset only where every row of its run
    /// does, and the two versions of a `trun` write those offsets unsigned and
    /// signed, so one run reaches either past [`i32::MAX`] or below zero — not
    /// both.
    fn takes(&self, offset: Option<i64>) -> bool {
        let Some(offset) = offset else {
            return !self.carries_offsets;
        };

        self.carries_offsets
            && !(offset.is_negative() && self.holds_wide_offset)
            && !(offset > i64::from(i32::MAX) && self.holds_negative_offset)
    }

    /// Adds `sample` to the run, and notes where its composition time offset falls
    fn push(&mut self, sample: PlacedSample) {
        if let Some(offset) = sample.sample_composition_time_offset {
            self.carries_offsets = true;
            self.holds_negative_offset |= offset.is_negative();
            self.holds_wide_offset |= offset > i64::from(i32::MAX);
        }

        self.samples.push(sample);
    }
}

/// Samples one track contributes to the fragment being written
///
/// `decode_time` is where the fragment places the track, which its `tfdt`
/// states, and `reached` where the samples added so far leave its media
/// timeline.
#[derive(Clone, Debug)]
struct OpenTrack {
    track_id: u32,
    decode_time: u64,
    reached: u64,
    sample_description_index: u32,
    runs: Vec<OpenRun>,
}

impl OpenTrack {
    /// Adds `sample` to this track, in the run it carries on or one starting at `data_offset`
    ///
    /// `carries_on` states whether the sample handed over before this one
    /// belonged to this track, which is what makes the two lie next to each
    /// other in the media data.
    fn place(
        &mut self,
        sample: PlacedSample,
        data_offset: u64,
        carries_on: bool,
    ) -> Result<(), SampleError> {
        match self.runs.last_mut() {
            Some(run) if carries_on && run.takes(sample.sample_composition_time_offset) => {
                run.push(sample);
            }
            _no_run_this_sample_carries_on => {
                let mut started = OpenRun {
                    data_offset,
                    samples: Vec::new(),
                    carries_offsets: false,
                    holds_negative_offset: false,
                    holds_wide_offset: false,
                };
                started.push(sample);
                self.runs.push(started);
            }
        }

        self.reached = self
            .reached
            .checked_add(u64::from(sample.sample_duration))
            .ok_or(SampleError::decode_time_overflow(self.track_id))?;

        Ok(())
    }
}

/// Fragment being written, holding its samples until it is closed
///
/// The tracks lie in the order they first appeared, which is the order their
/// `traf` boxes are written in, and `placed_tracks` names where each one lies.
#[derive(Clone, Debug)]
pub(super) struct OpenFragment {
    sequence_number: u32,
    tracks: Vec<OpenTrack>,
    placed_tracks: BTreeMap<u32, usize>,
    media_data: Vec<u8>,
    last_track_id: Option<u32>,
}

impl OpenFragment {
    /// Opens a fragment carrying no samples yet, which `mfhd` numbers `sequence_number`
    pub(super) const fn new(sequence_number: u32) -> Self {
        Self {
            sequence_number,
            tracks: Vec::new(),
            placed_tracks: BTreeMap::new(),
            media_data: Vec::new(),
            last_track_id: None,
        }
    }

    /// Places `sample` in the fragment
    ///
    /// The sample lands where it arrived: its bytes go on the end of the media
    /// data, and what it states is written by the `traf` of its track.
    /// `reached` is where the fragments closed before this one leave each
    /// track, which a track reaching this fragment for the first time is
    /// checked against.
    ///
    /// # Errors
    ///
    /// * [`DecodeTimeMismatch`](crate::SampleErrorKind::DecodeTimeMismatch),
    ///   [`BackwardDecodeTime`](crate::SampleErrorKind::BackwardDecodeTime),
    ///   [`SampleDescriptionIndexMismatch`](crate::SampleErrorKind::SampleDescriptionIndexMismatch),
    ///   [`OutOfRange`](crate::SampleErrorKind::OutOfRange) and
    ///   [`DecodeTimeOverflow`](crate::SampleErrorKind::DecodeTimeOverflow), as
    ///   [`SampleWriter::handle_sample`](super::SampleWriter::handle_sample)
    ///   states them.
    pub(super) fn place(
        &mut self,
        sample: Sample,
        reached: &BTreeMap<u32, u64>,
    ) -> Result<(), SampleError> {
        let track_id = sample.track_id();
        let offered = sample.data().len() as u64;
        let Ok(sample_size) = u32::try_from(offered) else {
            return Err(SampleError::sample_size_out_of_range(track_id, offered));
        };

        let placed = PlacedSample {
            sample_duration: sample.sample_duration(),
            sample_size,
            sample_flags: sample.sample_flags(),
            sample_composition_time_offset: sample.sample_composition_time_offset(),
        };
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
                    return Err(SampleError::sample_description_index_mismatch(
                        track_id,
                        sample_description_index,
                        track.sample_description_index,
                    ));
                }
                if track.reached != decode_time {
                    return Err(SampleError::decode_time_mismatch(
                        track_id,
                        decode_time,
                        track.reached,
                    ));
                }

                track.place(placed, data_offset, carries_on)?;
            }
            None => {
                if let Some(reached) = reached.get(&track_id).copied() {
                    if decode_time < reached {
                        return Err(SampleError::backward_decode_time(
                            track_id,
                            decode_time,
                            reached,
                        ));
                    }
                }

                let mut track = OpenTrack {
                    track_id,
                    decode_time,
                    reached: decode_time,
                    sample_description_index,
                    runs: Vec::new(),
                };
                track.place(placed, data_offset, false)?;
                self.placed_tracks.insert(track_id, self.tracks.len());
                self.tracks.push(track);
            }
        }

        self.media_data.extend_from_slice(sample.data());
        self.last_track_id = Some(track_id);

        Ok(())
    }

    /// Returns where the samples of the fragment leave the timeline of each track it carries
    pub(super) fn reached(&self) -> impl Iterator<Item = (u32, u64)> + use<'_> {
        self.tracks
            .iter()
            .map(|track| (track.track_id, track.reached))
    }

    /// Builds the boxes the fragment is written as, now that its samples are over
    ///
    /// # Errors
    ///
    /// * [`OutOfRange`](crate::SampleErrorKind::OutOfRange): a sample lies
    ///   further into the fragment than a `trun` reaches, or states a
    ///   composition time offset neither version of a `trun` writes.
    pub(super) fn into_boxes(self) -> Result<(MovieFragmentBox, MediaDataBox), SampleError> {
        let measured = build_movie_fragment(self.sequence_number, &self.tracks, None)?;
        let media_data = MediaDataBox::new(self.media_data);
        let header_len = media_data
            .encoded_len()
            .saturating_sub(media_data.data().len() as u64);
        let base = measured.encoded_len().saturating_add(header_len);
        let movie_fragment = build_movie_fragment(self.sequence_number, &self.tracks, Some(base))?;

        Ok((movie_fragment, media_data))
    }
}

/// What the samples of one track fragment share, and so what its `tfhd` states
///
/// A field every sample of the fragment states the same value for is written
/// once as the default of the `tfhd`, and left out of the rows of its runs. The
/// flags of the first sample are a default of their own (ISO/IEC 14496-12
/// §8.8.8), so a fragment whose samples share their flags but for the first one
/// states both.
#[derive(Clone, Copy, Debug)]
struct Defaults {
    sample_duration: Option<u32>,
    sample_size: Option<u32>,
    sample_flags: Option<u32>,
    first_sample_flags: Option<u32>,
}

impl Defaults {
    /// Returns what the samples of `track` share
    fn of(track: &OpenTrack) -> Self {
        let samples = || track.runs.iter().flat_map(|run| run.samples.iter());
        let flags = || samples().map(|sample| sample.sample_flags);

        let (sample_flags, first_sample_flags) = match shared(flags()) {
            Some(uniform) => (Some(uniform), None),
            None => match flags().next().zip(shared(flags().skip(1))) {
                Some((leading, trailing)) => (Some(trailing), Some(leading)),
                None => (None, None),
            },
        };

        Self {
            sample_duration: shared(samples().map(|sample| sample.sample_duration)),
            sample_size: shared(samples().map(|sample| sample.sample_size)),
            sample_flags,
            first_sample_flags,
        }
    }
}

/// Returns the value `values` holds throughout, where every one of them is that value
fn shared<Values: Iterator<Item = u32>>(mut values: Values) -> Option<u32> {
    let first = values.next()?;

    values.all(|value| value == first).then_some(first)
}

/// Builds the `moof` the samples of one fragment are written as
///
/// `base` is where the media data of the fragment lies, counted from the start
/// of the `moof` — the anchor `default-base-is-moof` establishes (ISO/IEC 14496-12
/// §8.8.7.1).
/// `None` builds the same boxes with every offset zero, which states the length
/// of the `moof` that those offsets are counted from.
// Why not measuring with a base of zero and dropping the `Option`: the offsets
// are checked against the field that carries them as they are built, and a
// fragment past that field would then be refused naming the offset it holds in
// the media data rather than the one that did not fit.
fn build_movie_fragment(
    sequence_number: u32,
    tracks: &[OpenTrack],
    base: Option<u64>,
) -> Result<MovieFragmentBox, SampleError> {
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
fn build_track_fragment(
    track: &OpenTrack,
    base: Option<u64>,
) -> Result<TrackFragmentBox, SampleError> {
    let defaults = Defaults::of(track);
    let header = TrackFragmentHeaderBox::new(
        TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF,
        track.track_id,
        None,
        Some(track.sample_description_index),
        defaults.sample_duration,
        defaults.sample_size,
        defaults.sample_flags,
    );
    let Some(header) = header else {
        // Why not unwrap: the only flags handed over are the anchor, and a
        // `tfhd` refuses none but the bits stating a field is present, so this
        // stands for a `None` the call does not reach.
        return Err(SampleError::fragment_not_representable());
    };

    let runs = track
        .runs
        .iter()
        .enumerate()
        .map(|(position, run)| build_track_run(track, run, &defaults, base, position == 0))
        .collect::<Result<Vec<_>, _>>()?;

    TrackFragmentBox::new(
        header,
        Some(TrackFragmentBaseMediaDecodeTimeBox::new(track.decode_time)),
        runs,
    )
    .ok_or_else(SampleError::fragment_not_representable)
}

/// Builds the `trun` one run of the samples of a track is written as
fn build_track_run(
    track: &OpenTrack,
    run: &OpenRun,
    defaults: &Defaults,
    base: Option<u64>,
    leads: bool,
) -> Result<TrackRunBox, SampleError> {
    let data_offset = match base {
        Some(base) => {
            let offset = base.checked_add(run.data_offset).ok_or_else(|| {
                SampleError::data_offset_out_of_range(track.track_id, run.data_offset)
            })?;

            i32::try_from(offset).map_err(|_past_the_field| {
                SampleError::data_offset_out_of_range(track.track_id, offset)
            })?
        }
        None => 0,
    };

    let rows = run
        .samples
        .iter()
        .map(|sample| {
            TrackRunSample::new(
                defaults
                    .sample_duration
                    .is_none()
                    .then_some(sample.sample_duration),
                defaults.sample_size.is_none().then_some(sample.sample_size),
                defaults
                    .sample_flags
                    .is_none()
                    .then_some(sample.sample_flags),
                sample.sample_composition_time_offset,
            )
            .ok_or_else(|| {
                SampleError::composition_time_offset_out_of_range(
                    track.track_id,
                    // Why not the offset alone: a row is refused for its
                    // composition time offset and nothing else, so the offset is
                    // there whenever this is reached, and the fallback stands for
                    // a `None` the call does not reach.
                    sample.sample_composition_time_offset.unwrap_or_default(),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let first_sample_flags = defaults.first_sample_flags.filter(|_leading| leads);

    TrackRunBox::new(Some(data_offset), first_sample_flags, rows)
        .ok_or_else(SampleError::fragment_not_representable)
}
