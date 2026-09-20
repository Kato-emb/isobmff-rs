//! [`OpenFragment`], the samples of one movie fragment held until it is closed

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use isobmff_boxes::{
    CompositionTimeOffset, MediaDataBox, MovieFragmentBox, MovieFragmentHeaderBox,
    TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentBox, TrackFragmentFlags,
    TrackFragmentHeaderBox, TrackRunBuilder, TrackRunRow,
};
use isobmff_core::BoxEncode as _;

use crate::error::SampleError;
use crate::sample::Sample;
use crate::track_decode_times::TrackDecodeTimes;

/// Samples of one track lying next to each other in the media data of a fragment
///
/// `data_offset` is where the run starts in that media data.
#[derive(Clone, Debug)]
struct OpenRun {
    data_offset: u64,
    rows: TrackRunBuilder,
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
    /// Adds `row` to this track, in the run it carries on or one starting at `data_offset`
    ///
    /// `carries_on` states whether the sample handed over before this one
    /// belonged to this track, which is what makes the two lie next to each
    /// other in the media data. A run that cannot write the row beside the
    /// ones it holds hands it back, and a new run starts with it.
    fn place(
        &mut self,
        row: TrackRunRow,
        data_offset: u64,
        carries_on: bool,
    ) -> Result<(), SampleError> {
        self.reached = self
            .reached
            .checked_add(u64::from(row.sample_duration()))
            .ok_or(SampleError::decode_time_overflow(self.track_id))?;

        let handed_back = match self.runs.last_mut() {
            Some(run) if carries_on => run.rows.push(row).err(),
            _no_run_this_sample_carries_on => Some(row),
        };
        if let Some(row) = handed_back {
            self.runs.push(OpenRun {
                data_offset,
                rows: TrackRunBuilder::new(row),
            });
        }

        Ok(())
    }

    /// Returns every row of every run of the track, in the order they were placed
    fn rows(&self) -> impl Iterator<Item = &TrackRunRow> {
        self.runs.iter().flat_map(|run| run.rows.rows())
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

    /// Places `sample` in the fragment, its bytes on the end of the media data
    ///
    /// `decode_times` is where the fragments closed before this one leave each
    /// track, which a track reaching this fragment for the first time is
    /// checked against.
    pub(super) fn place(
        &mut self,
        sample: Sample,
        decode_times: &TrackDecodeTimes,
    ) -> Result<(), SampleError> {
        let track_id = sample.track_id();
        let offered = sample.data().len() as u64;
        let Ok(sample_size) = u32::try_from(offered) else {
            return Err(SampleError::sample_size_out_of_range(track_id, offered));
        };
        let offset = sample.sample_composition_time_offset();
        let Some(sample_composition_time_offset) = CompositionTimeOffset::new(offset) else {
            return Err(SampleError::composition_time_offset_out_of_range(
                track_id, offset,
            ));
        };

        let row = TrackRunRow::new(
            sample.sample_duration(),
            sample_size,
            sample.sample_flags(),
            sample_composition_time_offset,
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

                track.place(row, data_offset, carries_on)?;
            }
            None => {
                let reached = decode_times.decode_time(track_id);
                if decode_time < reached {
                    return Err(SampleError::backward_decode_time(
                        track_id,
                        decode_time,
                        reached,
                    ));
                }

                let mut track = OpenTrack {
                    track_id,
                    decode_time,
                    reached: decode_time,
                    sample_description_index,
                    runs: Vec::new(),
                };
                track.place(row, data_offset, false)?;
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
    ) -> Result<(MovieFragmentBox, Vec<u8>), SampleError> {
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
/// once as the default of the `tfhd`, and left out of the rows of its runs. The
/// flags of the first sample are a default of their own (ISO/IEC 14496-12
/// §8.8.8), so a fragment whose samples share their flags but for the first one
/// states the shared ones.
#[derive(Clone, Copy, Debug)]
struct Defaults {
    sample_duration: Option<u32>,
    sample_size: Option<u32>,
    sample_flags: Option<u32>,
}

impl Defaults {
    /// Returns what the samples of `track` share
    fn of(track: &OpenTrack) -> Self {
        let flags = || track.rows().map(TrackRunRow::sample_flags);

        Self {
            sample_duration: shared(track.rows().map(TrackRunRow::sample_duration)),
            sample_size: shared(track.rows().map(TrackRunRow::sample_size)),
            sample_flags: shared(flags()).or_else(|| shared(flags().skip(1))),
        }
    }
}

/// Returns the value every item of `values` is, when they are all one value
fn shared<Values: Iterator<Item = u32>>(mut values: Values) -> Option<u32> {
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
) -> Result<MovieFragmentBox, SampleError> {
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
fn build_track_fragment(
    track: &OpenTrack,
    base: Option<u64>,
) -> Result<TrackFragmentBox, SampleError> {
    let defaults = Defaults::of(track);
    let header = TrackFragmentHeaderBox::new(
        TrackFragmentFlags::DEFAULT_BASE_IS_MOOF,
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
                        SampleError::data_offset_out_of_range(track.track_id, offset)
                    })?
                }
                None => 0,
            };
            Ok(run.rows.build(Some(data_offset), &header))
        })
        .collect::<Result<Vec<_>, SampleError>>()?;

    Ok(TrackFragmentBox::new(
        header,
        Some(TrackFragmentBaseMediaDecodeTimeBox::new(track.decode_time)),
        runs,
    ))
}
#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_boxes::{
        MovieFragmentBox, MovieFragmentHeaderBox, TrackFragmentBox, TrackFragmentFlags,
        TrackFragmentHeaderBox, TrackRunBox, TrackRunSample,
    };
    use isobmff_core::BoxEncode as _;

    use crate::error::SampleError;
    use crate::movie_fragment_writer::MovieFragmentWriter;
    use crate::movie_fragment_writer::tests::sample;
    use crate::sample::Sample;

    /// Bytes the header of the `mdat` beside a fragment occupies
    const MEDIA_DATA_HEADER_LEN: u64 = 8;

    /// Sample of track 1 at `decode_time` stating `sample_composition_time_offset`
    fn offset_by(decode_time: u64, sample_composition_time_offset: i64) -> Sample {
        Sample::new(
            1,
            decode_time,
            1_024,
            sample_composition_time_offset,
            0,
            1,
            b"AAAA".to_vec(),
        )
    }

    /// Sample of track 1 at `decode_time` stating `sample_flags`
    fn flagged(decode_time: u64, sample_flags: u32) -> Sample {
        Sample::new(1, decode_time, 1_024, 0, sample_flags, 1, b"AAAA".to_vec())
    }

    /// Writes `samples` as one fragment, and returns the boxes it is written as
    fn one_fragment(samples: Vec<Sample>) -> (MovieFragmentBox, Vec<u8>) {
        let mut writer = MovieFragmentWriter::new();

        writer.begin_fragment(1).unwrap();
        for sample in samples {
            writer.handle_sample(sample).unwrap();
        }

        writer.finish_fragment().unwrap()
    }

    /// The fragment `track_id` contributed to `movie_fragment`
    fn track_fragment_of(movie_fragment: &MovieFragmentBox, track_id: u32) -> &TrackFragmentBox {
        movie_fragment
            .traf()
            .iter()
            .find(|track_fragment| track_fragment.tfhd().track_id() == track_id)
            .unwrap()
    }

    /// Offset the first run of `movie_fragment` states, past the fragment and the header of its `mdat`
    fn data_offset_of(movie_fragment: &MovieFragmentBox) -> i32 {
        i32::try_from(
            movie_fragment
                .encoded_len()
                .saturating_add(MEDIA_DATA_HEADER_LEN),
        )
        .unwrap()
    }

    /// Rows each run of the fragment of `track_id` carries
    fn rows_of_the_runs(movie_fragment: &MovieFragmentBox, track_id: u32) -> Vec<usize> {
        track_fragment_of(movie_fragment, track_id)
            .trun()
            .iter()
            .map(|track_run| track_run.samples().len())
            .collect()
    }

    /// Header a `traf` of track 1 is written with, stating the defaults given
    fn track_fragment_header(
        default_sample_duration: Option<u32>,
        default_sample_size: Option<u32>,
        default_sample_flags: Option<u32>,
    ) -> TrackFragmentHeaderBox {
        TrackFragmentHeaderBox::new(
            TrackFragmentFlags::DEFAULT_BASE_IS_MOOF,
            1,
            None,
            Some(1),
            default_sample_duration,
            default_sample_size,
            default_sample_flags,
        )
    }

    /// Every row the fragment of `track_id` carries, run after run
    fn rows_of(movie_fragment: &MovieFragmentBox, track_id: u32) -> Vec<TrackRunSample> {
        track_fragment_of(movie_fragment, track_id)
            .trun()
            .iter()
            .flat_map(|track_run| track_run.samples())
            .cloned()
            .collect()
    }

    #[test]
    fn a_decode_time_is_written_for_every_fragment_of_a_track() {
        let mut writer = MovieFragmentWriter::new();
        let mut decode_times = Vec::new();

        for (sequence_number, decode_time) in [(1, 0), (2, 8_192)] {
            writer.begin_fragment(sequence_number).unwrap();
            writer
                .handle_sample(sample(1, decode_time, b"AAAA"))
                .unwrap();
            let (movie_fragment, _media_data) = writer.finish_fragment().unwrap();
            decode_times.push(
                track_fragment_of(&movie_fragment, 1)
                    .tfdt()
                    .unwrap()
                    .base_media_decode_time(),
            );
        }

        assert_eq!(decode_times, [0, 8_192]);
    }

    #[test]
    fn the_media_data_holds_the_samples_in_the_order_they_arrived() {
        let (_movie_fragment, media_data) = one_fragment(vec![
            sample(1, 0, b"AAAA"),
            sample(2, 0, b"BBBB"),
            sample(1, 1_024, b"CCCC"),
        ]);

        assert_eq!(media_data, b"AAAABBBBCCCC");
    }

    #[test]
    fn one_track_fragment_per_track_in_the_order_the_tracks_first_appear() {
        let (movie_fragment, _media_data) = one_fragment(vec![
            sample(2, 0, b"AAAA"),
            sample(1, 0, b"BBBB"),
            sample(2, 1_024, b"CCCC"),
        ]);

        let tracks: Vec<u32> = movie_fragment
            .traf()
            .iter()
            .map(|track_fragment| track_fragment.tfhd().track_id())
            .collect();
        assert_eq!(tracks, [2, 1]);
    }

    #[test]
    fn samples_of_one_track_handed_over_together_are_one_run() {
        let (together, _media_data) = one_fragment(vec![
            sample(1, 0, b"AAAA"),
            sample(1, 1_024, b"BBBB"),
            sample(2, 0, b"CCCC"),
        ]);
        let (apart, _media_data) = one_fragment(vec![
            sample(1, 0, b"AAAA"),
            sample(2, 0, b"CCCC"),
            sample(1, 1_024, b"BBBB"),
        ]);

        assert_eq!(rows_of_the_runs(&together, 1), [2]);
        assert_eq!(rows_of_the_runs(&apart, 1), [1, 1]);
    }

    #[test]
    fn offsets_are_anchored_at_the_fragment_itself() {
        let (movie_fragment, _media_data) =
            one_fragment(vec![sample(1, 0, b"AAAA"), sample(2, 0, b"BBBB")]);

        let past_the_fragment = movie_fragment
            .encoded_len()
            .saturating_add(MEDIA_DATA_HEADER_LEN);
        let offsets: Vec<Option<i32>> = movie_fragment
            .traf()
            .iter()
            .flat_map(TrackFragmentBox::trun)
            .map(TrackRunBox::data_offset)
            .collect();

        assert_eq!(
            offsets,
            [
                Some(i32::try_from(past_the_fragment).unwrap()),
                Some(i32::try_from(past_the_fragment.saturating_add(4)).unwrap()),
            ]
        );
    }

    #[test]
    fn what_the_samples_share_is_stated_once_by_their_track_fragment_header() {
        let (movie_fragment, _media_data) =
            one_fragment(vec![sample(1, 0, b"AAAA"), sample(1, 1_024, b"BBBB")]);
        let header = track_fragment_of(&movie_fragment, 1).tfhd();

        assert_eq!(
            *header,
            track_fragment_header(Some(1_024), Some(4), Some(0))
        );
        assert_eq!(
            rows_of(&movie_fragment, 1),
            vec![TrackRunSample::new(None, None, None, None).unwrap(); 2]
        );
    }

    #[test]
    fn what_the_samples_do_not_share_is_stated_by_every_row() {
        let shorter = Sample::new(1, 1_024, 512, 0, 0, 1, b"BB".to_vec());
        let (movie_fragment, _media_data) = one_fragment(vec![sample(1, 0, b"AAAA"), shorter]);
        let header = track_fragment_of(&movie_fragment, 1).tfhd();

        assert_eq!(*header, track_fragment_header(None, None, Some(0)));
        assert_eq!(
            rows_of(&movie_fragment, 1),
            [
                TrackRunSample::new(Some(1_024), Some(4), None, None).unwrap(),
                TrackRunSample::new(Some(512), Some(2), None, None).unwrap(),
            ]
        );
    }

    #[test]
    fn flags_only_the_first_sample_differs_on_are_written_as_its_own() {
        let (movie_fragment, _media_data) = one_fragment(vec![
            flagged(0, 0x0200_0000),
            flagged(1_024, 0x0101_0000),
            flagged(2_048, 0x0101_0000),
        ]);
        let track_fragment = track_fragment_of(&movie_fragment, 1);

        assert_eq!(
            *track_fragment.tfhd(),
            track_fragment_header(Some(1_024), Some(4), Some(0x0101_0000))
        );
        assert_eq!(
            track_fragment.trun(),
            [TrackRunBox::new(
                Some(data_offset_of(&movie_fragment)),
                Some(0x0200_0000),
                vec![TrackRunSample::new(None, None, None, None).unwrap(); 3],
            )
            .unwrap()]
        );
    }

    #[test]
    fn flags_no_two_samples_share_are_written_by_every_row() {
        let (movie_fragment, _media_data) = one_fragment(vec![
            flagged(0, 0x0200_0000),
            flagged(1_024, 0x0101_0000),
            flagged(2_048, 0x0100_0000),
        ]);
        let track_fragment = track_fragment_of(&movie_fragment, 1);

        assert_eq!(
            *track_fragment.tfhd(),
            track_fragment_header(Some(1_024), Some(4), None)
        );
        assert_eq!(
            track_fragment.trun(),
            [TrackRunBox::new(
                Some(data_offset_of(&movie_fragment)),
                None,
                vec![
                    TrackRunSample::new(None, None, Some(0x0200_0000), None).unwrap(),
                    TrackRunSample::new(None, None, Some(0x0101_0000), None).unwrap(),
                    TrackRunSample::new(None, None, Some(0x0100_0000), None).unwrap(),
                ],
            )
            .unwrap()]
        );
    }

    #[test]
    fn a_run_holding_any_composition_time_offset_states_one_for_every_row() {
        let (movie_fragment, _media_data) =
            one_fragment(vec![sample(1, 0, b"AAAA"), offset_by(1_024, 8)]);

        assert_eq!(rows_of_the_runs(&movie_fragment, 1), [2]);
        assert_eq!(
            rows_of(&movie_fragment, 1),
            [
                TrackRunSample::new(None, None, None, Some(0)).unwrap(),
                TrackRunSample::new(None, None, None, Some(8)).unwrap(),
            ]
        );
    }

    #[test]
    fn a_run_is_split_where_no_trun_version_writes_both_offsets() {
        let (movie_fragment, _media_data) = one_fragment(vec![
            offset_by(0, -8),
            offset_by(1_024, i64::from(u32::MAX)),
        ]);

        assert_eq!(rows_of_the_runs(&movie_fragment, 1), [1, 1]);
    }

    #[test]
    fn a_composition_time_offset_no_run_writes_is_refused() {
        let past_the_field = i64::from(u32::MAX).saturating_add(1);
        let mut writer = MovieFragmentWriter::new();

        writer.begin_fragment(1).unwrap();

        assert_eq!(
            writer.handle_sample(offset_by(0, past_the_field)),
            Err(SampleError::composition_time_offset_out_of_range(
                1,
                past_the_field
            ))
        );
    }

    #[test]
    fn a_fragment_of_no_samples_is_written_as_an_empty_pair() {
        let (movie_fragment, media_data) = one_fragment(vec![]);

        assert_eq!(
            movie_fragment,
            MovieFragmentBox::new(MovieFragmentHeaderBox::new(1), vec![])
        );
        assert_eq!(media_data, Vec::<u8>::new());
    }

    #[test]
    fn a_sample_that_does_not_start_where_the_one_before_it_ends_is_refused() {
        let mut writer = MovieFragmentWriter::new();

        writer.begin_fragment(1).unwrap();
        writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();

        assert_eq!(
            writer.handle_sample(sample(1, 512, b"BBBB")),
            Err(SampleError::decode_time_mismatch(1, 512, 1_024))
        );
    }

    #[test]
    fn samples_of_one_fragment_described_by_two_entries_are_refused() {
        let described_by_the_second = Sample::new(1, 1_024, 1_024, 0, 0, 2, b"BBBB".to_vec());
        let mut writer = MovieFragmentWriter::new();

        writer.begin_fragment(1).unwrap();
        writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();

        assert_eq!(
            writer.handle_sample(described_by_the_second),
            Err(SampleError::sample_description_index_mismatch(1, 2, 1))
        );
    }

    #[test]
    fn decode_times_running_past_what_64_bits_carry_are_refused() {
        let at_the_end_of_time = Sample::new(1, u64::MAX, 1_024, 0, 0, 1, b"AAAA".to_vec());
        let mut writer = MovieFragmentWriter::new();

        writer.begin_fragment(1).unwrap();

        assert_eq!(
            writer.handle_sample(at_the_end_of_time),
            Err(SampleError::decode_time_overflow(1))
        );
    }
}
