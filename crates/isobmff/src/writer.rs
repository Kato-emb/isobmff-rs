//! [`SampleWriter`], the samples of a presentation laid out as movie fragments

mod fragment;

use alloc::collections::{BTreeMap, VecDeque};
use core::mem;

use isobmff_boxes::{MediaDataBox, MovieFragmentBox};

use crate::error::SampleError;
use crate::sample::Sample;
use crate::writer::fragment::OpenFragment;

/// Where the writer stands between calls
#[derive(Clone, Debug)]
enum State {
    /// Between fragments, waiting for the next one to be opened
    Between,
    /// Laying out a fragment that was opened, and taking the samples it carries
    Fragment(OpenFragment),
    /// Told the samples are over, and taking nothing more
    Finished,
    /// Failed, and reporting that same failure for every call after it
    Failed(SampleError),
}

/// Lays the samples of a presentation out as movie fragments
///
/// The writer takes the samples of a fragment between
/// [`begin_fragment`](Self::begin_fragment) and
/// [`finish_fragment`](Self::finish_fragment), and reports the `moof` and the
/// `mdat` they make as a pair from [`poll_fragment`](Self::poll_fragment). It
/// writes nothing itself: what the two boxes are laid down as, and where, stay
/// with the caller — [`FragmentedWriter`](crate::FragmentedWriter) lays them
/// down as a fragmented file, and [`BoxEncode`](isobmff_core::BoxEncode) writes
/// either on its own.
///
/// The brands and the movie the fragments continue are the caller's too, and the
/// `trex` of a track sets defaults this writer never leans on — every default a
/// fragment falls back on is stated by its own `tfhd`.
///
/// # Layout
///
/// The order the samples arrive in is the only order the writer has, so it is
/// the one it lays them out in.
///
/// * The media data holds the samples in the order they were handed over. A
///   caller interleaving two tracks states that by handing them over
///   interleaved.
/// * One `traf` per track, in the order the tracks first appear. A run of
///   samples of one track handed over together is one `trun`.
/// * A fragment therefore declares its samples track by track, whatever order
///   they were handed over in — the media data holds that order, the `traf`
///   boxes group it. A [`SampleReader`](crate::SampleReader) reports the samples
///   of a fragment in the order it declares them, so samples of two tracks
///   handed over interleaved are read back grouped by track.
/// * Offsets are anchored at the fragment itself
///   ([`default-base-is-moof`](isobmff_boxes::TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF),
///   ISO/IEC 14496-12 §8.8.7.1), and every `trun` states its own. They are counted over the
///   `moof` and the header of the `mdat`, so the two are laid down as they come
///   out: the media data of a fragment directly after the fragment itself.
/// * A `tfdt` is always written, stating the decode time of the first sample of
///   its `traf`.
/// * What the samples of a `traf` share — how long they last, how long they
///   are, their flags — is written once as a default of its `tfhd`, and left
///   out of the rows. What they do not share is stated per row, except flags
///   that only the first sample differs on, which are its
///   `first_sample_flags`. The `stsd` entry is always stated by the `tfhd`.
///
/// # Contract
///
/// * A fragment is opened by [`begin_fragment`](Self::begin_fragment) and
///   closed by [`finish_fragment`](Self::finish_fragment). Handing a sample
///   over or closing a fragment while none is open is
///   [`NoFragmentOpen`](crate::SampleErrorKind::NoFragmentOpen), and opening
///   one or declaring the samples over while one is open is
///   [`FragmentStillOpen`](crate::SampleErrorKind::FragmentStillOpen).
/// * The `sequence_number` of a fragment is the caller's. §8.8.5 has it
///   increase over the fragments of a presentation, which the writer neither
///   checks nor reports.
/// * Within one fragment, a sample of a track starts where the one before it
///   ends: a `trun` states how long a sample lasts and not when it is decoded,
///   so a gap is
///   [`DecodeTimeMismatch`](crate::SampleErrorKind::DecodeTimeMismatch).
///   Between fragments a gap is written as it stands — the `tfdt` states it —
///   but a track never goes back, which is
///   [`BackwardDecodeTime`](crate::SampleErrorKind::BackwardDecodeTime).
/// * A gap between fragments is read back as the durations of this writer
///   report them, and §8.8.12 has a reader close it by extending the duration
///   of the sample before it instead. What the samples last therefore depends
///   on which reader reads them.
/// * The samples of one `traf` are all described by one `stsd` entry, which the
///   `tfhd` states for them: a fragment mixing two is
///   [`SampleDescriptionIndexMismatch`](crate::SampleErrorKind::SampleDescriptionIndexMismatch).
/// * A fragment of no samples is written as a `moof` of no `traf` beside an
///   empty `mdat`.
/// * The caller drains before handing over more. Closed fragments are held
///   until they are taken, so writing on without polling has the writer hold
///   the whole presentation.
/// * An `Err` leaves the writer failed for good,
///   [`AlreadyFinished`](crate::SampleErrorKind::AlreadyFinished) aside: every
///   later call reports that same failure again.
/// * [`finish`](Self::finish) declares the samples over. Fragments closed
///   before it are still taken after it, but anything handed over then, or a
///   second [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::SampleErrorKind::AlreadyFinished).
///
/// An empty `traf` stating a `tfdt` alone, which §8.8.12 allows for
/// establishing the duration of the sample before it, is not written: a track
/// reaches a fragment only by a sample of it.
///
/// # Examples
///
/// ```
/// use isobmff::{BoxEncode, Sample, SampleWriter};
///
/// // A writer waiting for the first fragment
/// let mut writer = SampleWriter::new();
///
/// // One fragment of two samples of track 1, lasting 1024 units each
/// writer.begin_fragment(1).unwrap();
/// writer
///     .handle_sample(Sample::new(1, 0, 1_024, None, 0, 1, b"SAMP".to_vec()))
///     .unwrap();
/// writer
///     .handle_sample(Sample::new(1, 1_024, 1_024, None, 0, 1, b"DATA".to_vec()))
///     .unwrap();
/// writer.finish_fragment().unwrap();
/// writer.finish().unwrap();
///
/// // The fragment and the media data beside it come out as a pair
/// let (movie_fragment, media_data) = writer.poll_fragment().unwrap();
/// assert_eq!(media_data.data(), b"SAMPDATA");
///
/// // The samples share how long they last, so their `tfhd` states it for both
/// let track_fragment = movie_fragment.traf().first().unwrap();
/// assert_eq!(track_fragment.tfhd().default_sample_duration(), Some(1_024));
/// assert_eq!(
///     track_fragment.tfdt().unwrap().base_media_decode_time(),
///     0
/// );
///
/// // Their data lies past the fragment and the header of the `mdat` beside it
/// let track_run = track_fragment.trun().first().unwrap();
/// let past_the_fragment = movie_fragment.encoded_len() + 8;
/// assert_eq!(
///     track_run.data_offset(),
///     Some(i32::try_from(past_the_fragment).unwrap())
/// );
/// ```
#[derive(Debug)]
pub struct SampleWriter {
    fragments: VecDeque<(MovieFragmentBox, MediaDataBox)>,
    reached: BTreeMap<u32, u64>,
    state: State,
}

impl SampleWriter {
    /// Creates a writer waiting for the first fragment
    #[must_use]
    pub const fn new() -> Self {
        Self {
            fragments: VecDeque::new(),
            reached: BTreeMap::new(),
            state: State::Between,
        }
    }

    /// Opens a fragment, which the samples handed over next are laid out in
    ///
    /// `sequence_number` is what its `mfhd` states. §8.8.5 has it increase over
    /// the fragments of a presentation, which is the caller's to hold to.
    ///
    /// # Errors
    ///
    /// * [`FragmentStillOpen`](crate::SampleErrorKind::FragmentStillOpen): the
    ///   fragment before it was not closed.
    /// * [`AlreadyFinished`](crate::SampleErrorKind::AlreadyFinished): the
    ///   samples were declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn begin_fragment(&mut self, sequence_number: u32) -> Result<(), SampleError> {
        self.writing()?;

        if matches!(self.state, State::Fragment(_)) {
            return Err(self.fail(SampleError::fragment_still_open()));
        }
        self.state = State::Fragment(OpenFragment::new(sequence_number));

        Ok(())
    }

    /// Takes a sample, and places it in the fragment that is open
    ///
    /// The sample lands where it arrived: its bytes go on the end of the media
    /// data of the fragment, and what it states is written by the `traf` of its
    /// track.
    ///
    /// # Errors
    ///
    /// * [`NoFragmentOpen`](crate::SampleErrorKind::NoFragmentOpen): no
    ///   fragment was opened to carry it.
    /// * [`DecodeTimeMismatch`](crate::SampleErrorKind::DecodeTimeMismatch):
    ///   the sample does not start where the one before it in its track ends.
    /// * [`BackwardDecodeTime`](crate::SampleErrorKind::BackwardDecodeTime):
    ///   the sample starts before the samples written for its track reach.
    /// * [`SampleDescriptionIndexMismatch`](crate::SampleErrorKind::SampleDescriptionIndexMismatch):
    ///   the sample is described by another `stsd` entry than its fragment
    ///   states for the track.
    /// * [`OutOfRange`](crate::SampleErrorKind::OutOfRange): the sample is
    ///   longer than a `trun` row states.
    /// * [`DecodeTimeOverflow`](crate::SampleErrorKind::DecodeTimeOverflow):
    ///   the decode times of its track run past what 64 bits carry.
    /// * [`AlreadyFinished`](crate::SampleErrorKind::AlreadyFinished): the
    ///   samples were declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), SampleError> {
        self.writing()?;

        self.place(sample).map_err(|failure| self.fail(failure))
    }

    /// Closes the fragment that is open, and makes the boxes it is written as
    ///
    /// The `moof` and the `mdat` are settled here, both held whole, so no offset
    /// is written before the length it counts from is known. They are then taken
    /// from [`poll_fragment`](Self::poll_fragment).
    ///
    /// # Errors
    ///
    /// * [`NoFragmentOpen`](crate::SampleErrorKind::NoFragmentOpen): no
    ///   fragment was open to close.
    /// * [`OutOfRange`](crate::SampleErrorKind::OutOfRange): a sample lies
    ///   further into the fragment than a `trun` reaches, or states a
    ///   composition time offset neither version of a `trun` writes.
    /// * [`AlreadyFinished`](crate::SampleErrorKind::AlreadyFinished): the
    ///   samples were declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn finish_fragment(&mut self) -> Result<(), SampleError> {
        self.writing()?;

        self.close().map_err(|failure| self.fail(failure))
    }

    /// Takes the next fragment that was closed, as the boxes it is written as
    ///
    /// Reports `None` once they are used up: more samples are needed. Failure is
    /// reported by the calls that take the samples, so this one never fails — a
    /// failed writer hands over the fragments it had already closed, then `None`
    /// from there on.
    pub fn poll_fragment(&mut self) -> Option<(MovieFragmentBox, MediaDataBox)> {
        self.fragments.pop_front()
    }

    /// Declares the samples over
    ///
    /// # Errors
    ///
    /// * [`FragmentStillOpen`](crate::SampleErrorKind::FragmentStillOpen): a
    ///   fragment was left open.
    /// * [`AlreadyFinished`](crate::SampleErrorKind::AlreadyFinished): the
    ///   samples were already declared over.
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<(), SampleError> {
        self.writing()?;

        if matches!(self.state, State::Fragment(_)) {
            return Err(self.fail(SampleError::fragment_still_open()));
        }
        self.state = State::Finished;

        Ok(())
    }

    /// Returns `Ok` while the writer still takes samples
    fn writing(&self) -> Result<(), SampleError> {
        match self.state {
            State::Between | State::Fragment(_) => Ok(()),
            State::Finished => Err(SampleError::already_finished()),
            State::Failed(failure) => Err(failure),
        }
    }

    /// Places `sample` in the fragment that is open
    fn place(&mut self, sample: Sample) -> Result<(), SampleError> {
        let State::Fragment(open) = &mut self.state else {
            return Err(SampleError::no_fragment_open());
        };

        open.place(sample, &self.reached)
    }

    /// Closes the fragment that is open, and holds the boxes it is written as
    fn close(&mut self) -> Result<(), SampleError> {
        // Why not leaving the state alone until the fragment is known to build:
        // the boxes are built from the fragment whole, and the caller reached
        // here through `writing`, so the only state this replaces without a
        // fragment to take is the `Between` it puts back.
        let State::Fragment(open) = mem::replace(&mut self.state, State::Between) else {
            return Err(SampleError::no_fragment_open());
        };

        self.reached.extend(open.reached());
        self.fragments.push_back(open.into_boxes()?);

        Ok(())
    }

    /// Fails the writer for good, and hands the failure back to report
    fn fail(&mut self, failure: SampleError) -> SampleError {
        self.state = State::Failed(failure);

        failure
    }
}

impl Default for SampleWriter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_boxes::{
        MovieFragmentHeaderBox, TrackFragmentBox, TrackFragmentHeaderBox, TrackRunBox,
        TrackRunSample,
    };
    use isobmff_core::BoxEncode as _;

    use super::{MediaDataBox, MovieFragmentBox, Sample, SampleError, SampleWriter};

    /// Bytes the header of the `mdat` beside a fragment occupies
    const MEDIA_DATA_HEADER_LEN: u64 = 8;

    /// Sample of `track_id` at `decode_time`, lasting 1024 units and carrying `data`
    fn sample(track_id: u32, decode_time: u64, data: &[u8]) -> Sample {
        Sample::new(track_id, decode_time, 1_024, None, 0, 1, data.to_vec())
    }

    /// Sample of track 1 at `decode_time` stating `sample_composition_time_offset`
    fn offset_by(decode_time: u64, sample_composition_time_offset: i64) -> Sample {
        Sample::new(
            1,
            decode_time,
            1_024,
            Some(sample_composition_time_offset),
            0,
            1,
            b"AAAA".to_vec(),
        )
    }

    /// Sample of track 1 at `decode_time` stating `sample_flags`
    fn flagged(decode_time: u64, sample_flags: u32) -> Sample {
        Sample::new(
            1,
            decode_time,
            1_024,
            None,
            sample_flags,
            1,
            b"AAAA".to_vec(),
        )
    }

    /// Writes `samples` as one fragment, and returns the boxes it is written as
    fn one_fragment(samples: Vec<Sample>) -> (MovieFragmentBox, MediaDataBox) {
        let mut writer = SampleWriter::new();

        writer.begin_fragment(1).unwrap();
        for sample in samples {
            writer.handle_sample(sample).unwrap();
        }
        writer.finish_fragment().unwrap();
        writer.finish().unwrap();

        writer.poll_fragment().unwrap()
    }

    /// The fragment `track_id` contributed to `movie_fragment`
    fn track_fragment_of(movie_fragment: &MovieFragmentBox, track_id: u32) -> &TrackFragmentBox {
        movie_fragment
            .traf()
            .iter()
            .find(|track_fragment| track_fragment.tfhd().track_id() == track_id)
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
            TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF,
            1,
            None,
            Some(1),
            default_sample_duration,
            default_sample_size,
            default_sample_flags,
        )
        .unwrap()
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
    fn the_media_data_holds_the_samples_in_the_order_they_arrived() {
        let (_movie_fragment, media_data) = one_fragment(vec![
            sample(1, 0, b"AAAA"),
            sample(2, 0, b"BBBB"),
            sample(1, 1_024, b"CCCC"),
        ]);

        assert_eq!(media_data.data(), b"AAAABBBBCCCC");
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
    fn a_decode_time_is_written_for_every_fragment_of_a_track() {
        let mut writer = SampleWriter::new();

        for (sequence_number, decode_time) in [(1, 0), (2, 8_192)] {
            writer.begin_fragment(sequence_number).unwrap();
            writer
                .handle_sample(sample(1, decode_time, b"AAAA"))
                .unwrap();
            writer.finish_fragment().unwrap();
        }
        writer.finish().unwrap();

        let mut decode_times = Vec::new();
        while let Some((movie_fragment, _media_data)) = writer.poll_fragment() {
            let decode_time = track_fragment_of(&movie_fragment, 1)
                .tfdt()
                .unwrap()
                .base_media_decode_time();
            decode_times.push(decode_time);
        }

        assert_eq!(decode_times, [0, 8_192]);
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
        let shorter = Sample::new(1, 1_024, 512, None, 0, 1, b"BB".to_vec());
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
            track_fragment.tfhd().default_sample_flags(),
            Some(0x0101_0000)
        );
        assert_eq!(
            track_fragment.trun().first().unwrap().first_sample_flags(),
            Some(0x0200_0000)
        );
        assert_eq!(
            rows_of(&movie_fragment, 1),
            vec![TrackRunSample::new(None, None, None, None).unwrap(); 3]
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

        assert_eq!(track_fragment.tfhd().default_sample_flags(), None);
        assert_eq!(
            track_fragment.trun().first().unwrap().first_sample_flags(),
            None
        );
        assert_eq!(
            rows_of(&movie_fragment, 1),
            [
                TrackRunSample::new(None, None, Some(0x0200_0000), None).unwrap(),
                TrackRunSample::new(None, None, Some(0x0101_0000), None).unwrap(),
                TrackRunSample::new(None, None, Some(0x0100_0000), None).unwrap(),
            ]
        );
    }

    #[test]
    fn a_run_is_split_where_the_samples_start_stating_composition_time_offsets() {
        let (movie_fragment, _media_data) =
            one_fragment(vec![sample(1, 0, b"AAAA"), offset_by(1_024, 8)]);

        assert_eq!(rows_of_the_runs(&movie_fragment, 1), [1, 1]);
        assert_eq!(
            rows_of(&movie_fragment, 1),
            [
                TrackRunSample::new(None, None, None, None).unwrap(),
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
        let mut writer = SampleWriter::new();

        writer.begin_fragment(1).unwrap();
        writer.handle_sample(offset_by(0, past_the_field)).unwrap();

        assert_eq!(
            writer.finish_fragment(),
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
        assert_eq!(media_data, MediaDataBox::new(vec![]));
    }

    #[test]
    fn a_sample_that_does_not_start_where_the_one_before_it_ends_is_refused() {
        let mut writer = SampleWriter::new();

        writer.begin_fragment(1).unwrap();
        writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();

        assert_eq!(
            writer.handle_sample(sample(1, 512, b"BBBB")),
            Err(SampleError::decode_time_mismatch(1, 512, 1_024))
        );
    }

    #[test]
    fn a_fragment_may_start_after_the_samples_before_it_end() {
        let mut writer = SampleWriter::new();

        writer.begin_fragment(1).unwrap();
        writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();
        writer.finish_fragment().unwrap();
        writer.begin_fragment(2).unwrap();

        assert_eq!(writer.handle_sample(sample(1, 8_192, b"BBBB")), Ok(()));
    }

    #[test]
    fn a_track_going_back_to_a_decode_time_it_passed_is_refused() {
        let mut writer = SampleWriter::new();

        writer.begin_fragment(1).unwrap();
        writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();
        writer.finish_fragment().unwrap();
        writer.begin_fragment(2).unwrap();

        assert_eq!(
            writer.handle_sample(sample(1, 512, b"BBBB")),
            Err(SampleError::backward_decode_time(1, 512, 1_024))
        );
    }

    #[test]
    fn samples_of_one_fragment_described_by_two_entries_are_refused() {
        let described_by_the_second = Sample::new(1, 1_024, 1_024, None, 0, 2, b"BBBB".to_vec());
        let mut writer = SampleWriter::new();

        writer.begin_fragment(1).unwrap();
        writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();

        assert_eq!(
            writer.handle_sample(described_by_the_second),
            Err(SampleError::sample_description_index_mismatch(1, 2, 1))
        );
    }

    #[test]
    fn decode_times_running_past_what_64_bits_carry_are_refused() {
        let at_the_end_of_time = Sample::new(1, u64::MAX, 1_024, None, 0, 1, b"AAAA".to_vec());
        let mut writer = SampleWriter::new();

        writer.begin_fragment(1).unwrap();

        assert_eq!(
            writer.handle_sample(at_the_end_of_time),
            Err(SampleError::decode_time_overflow(1))
        );
    }

    #[test]
    fn a_sample_handed_over_while_no_fragment_is_open_is_refused() {
        let mut writer = SampleWriter::new();

        assert_eq!(
            writer.handle_sample(sample(1, 0, b"AAAA")),
            Err(SampleError::no_fragment_open())
        );
    }

    #[test]
    fn closing_a_fragment_while_none_is_open_is_refused() {
        let mut writer = SampleWriter::new();

        assert_eq!(
            writer.finish_fragment(),
            Err(SampleError::no_fragment_open())
        );
    }

    #[test]
    fn a_fragment_begun_while_one_is_open_is_refused() {
        let mut writer = SampleWriter::new();
        writer.begin_fragment(1).unwrap();

        assert_eq!(
            writer.begin_fragment(2),
            Err(SampleError::fragment_still_open())
        );
    }

    #[test]
    fn declaring_the_samples_over_while_a_fragment_is_open_is_refused() {
        let mut writer = SampleWriter::new();
        writer.begin_fragment(1).unwrap();

        assert_eq!(writer.finish(), Err(SampleError::fragment_still_open()));
    }

    #[test]
    fn a_fragment_is_closed_while_the_one_before_it_is_still_held() {
        let mut writer = SampleWriter::new();

        for (sequence_number, decode_time) in [(1, 0), (2, 1_024)] {
            writer.begin_fragment(sequence_number).unwrap();
            writer
                .handle_sample(sample(1, decode_time, b"AAAA"))
                .unwrap();
            writer.finish_fragment().unwrap();
        }
        writer.finish().unwrap();

        let sequence_numbers: Vec<u32> = core::iter::from_fn(|| writer.poll_fragment())
            .map(|(movie_fragment, _media_data)| movie_fragment.mfhd().sequence_number())
            .collect();

        assert_eq!(sequence_numbers, [1, 2]);
    }

    #[test]
    fn anything_handed_over_after_the_samples_were_declared_over_is_refused() {
        let mut writer = SampleWriter::new();
        writer.finish().unwrap();

        assert_eq!(
            writer.begin_fragment(1),
            Err(SampleError::already_finished())
        );
        assert_eq!(
            writer.handle_sample(sample(1, 0, b"AAAA")),
            Err(SampleError::already_finished())
        );
        assert_eq!(writer.finish(), Err(SampleError::already_finished()));
    }

    #[test]
    fn a_failure_is_reported_again_for_every_call_after_it() {
        let mut writer = SampleWriter::new();
        let failure = writer.handle_sample(sample(1, 0, b"AAAA")).unwrap_err();

        assert_eq!(writer.begin_fragment(1), Err(failure));
        assert_eq!(writer.finish_fragment(), Err(failure));
        assert_eq!(writer.finish(), Err(failure));
    }

    #[test]
    fn fragments_closed_before_a_failure_are_still_taken() {
        let mut writer = SampleWriter::new();

        writer.begin_fragment(1).unwrap();
        writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();
        writer.finish_fragment().unwrap();
        writer.finish_fragment().unwrap_err();

        let (_movie_fragment, media_data) = writer.poll_fragment().unwrap();
        assert_eq!(media_data.data(), b"AAAA");
        assert_eq!(writer.poll_fragment(), None);
    }
}
