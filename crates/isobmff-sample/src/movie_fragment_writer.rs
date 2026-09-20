//! [`MovieFragmentWriter`], the samples of a presentation laid out as movie fragments, ISO/IEC 14496-12 §8.8

mod open_fragment;

use alloc::vec::Vec;
use core::mem;

use isobmff_boxes::MovieFragmentBox;

use crate::error::SampleError;
use crate::movie_fragment_writer::open_fragment::OpenFragment;
use crate::sample::Sample;
use crate::track_decode_times::TrackDecodeTimes;

/// Lays the samples of a presentation out as movie fragments
///
/// The writer takes the samples of a fragment between
/// [`begin_fragment`](Self::begin_fragment) and
/// [`finish_fragment`](Self::finish_fragment), which hands back the `moof`
/// they are declared by and the payload of the `mdat` that carries them. It
/// writes nothing itself: what the two are laid down as, and where, stay with
/// the caller.
///
/// The brands and the movie the fragments continue are the caller's too, and
/// the `trex` of a track sets defaults this writer never leans on — every
/// default a fragment falls back on is stated by its own `tfhd`.
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
///   boxes group it.
///   [`movie_fragment::sample_extents`](crate::movie_fragment::sample_extents)
///   resolves the samples of a fragment in the order it declares them, so
///   samples of two tracks handed over interleaved come back grouped by track.
/// * Offsets are anchored at the fragment itself
///   ([`default-base-is-moof`](isobmff_boxes::TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF),
///   ISO/IEC 14496-12 §8.8.7.1), and every `trun` states its own. They are
///   counted over the `moof` and the header of the `mdat`, so the two are laid
///   down as they come out: the media data of a fragment directly after the
///   fragment itself.
/// * A `tfdt` is always written, stating the decode time of the first sample
///   of its `traf`.
/// * What the samples of a `traf` share — how long they last, how long they
///   are, their flags — is written once as a default of its `tfhd`, and left
///   out of the rows. What they do not share is stated per row, except flags
///   that only the first sample differs on, which are its
///   `first_sample_flags`. The `stsd` entry is always stated by the `tfhd`.
/// * A run whose samples all compose when they are decoded states no
///   composition time offset; one holding any other offset states one for
///   every row, and is cut where the two versions of a `trun` would disagree
///   on how to write them (§8.8.8).
///
/// # Contract
///
/// * A fragment is opened by [`begin_fragment`](Self::begin_fragment) and
///   closed by [`finish_fragment`](Self::finish_fragment). Handing a sample
///   over or closing a fragment while none is open is
///   [`NoFragmentOpen`](crate::SampleErrorKind::NoFragmentOpen), and opening
///   one while one is open is
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
/// * The samples of one `traf` are all described by one `stsd` entry, which
///   the `tfhd` states for them: a fragment mixing two is
///   [`SampleDescriptionIndexMismatch`](crate::SampleErrorKind::SampleDescriptionIndexMismatch).
/// * A fragment of no samples is written as a `moof` of no `traf` beside an
///   empty payload.
/// * An `Err` leaves the writer failed for good: every later call reports
///   that same failure again.
///
/// An empty `traf` stating a `tfdt` alone, which §8.8.12 allows for
/// establishing the duration of the sample before it, is not written: a track
/// reaches a fragment only by a sample of it.
///
/// # Examples
///
/// ```
/// use isobmff_core::BoxEncode as _;
/// use isobmff_sample::{MovieFragmentWriter, Sample};
///
/// let mut writer = MovieFragmentWriter::new();
///
/// // One fragment of two samples of track 1, lasting 1024 units each
/// writer.begin_fragment(1)?;
/// writer.handle_sample(Sample::new(1, 0, 1_024, 0, 0, 1, b"SAMP".to_vec()))?;
/// writer.handle_sample(Sample::new(1, 1_024, 1_024, 0, 0, 1, b"DATA".to_vec()))?;
/// let (movie_fragment, media_data) = writer.finish_fragment()?;
/// assert_eq!(media_data, b"SAMPDATA");
///
/// // The samples share how long they last, so their `tfhd` states it for both
/// let track_fragment = movie_fragment.traf().first().unwrap();
/// assert_eq!(track_fragment.tfhd().default_sample_duration(), Some(1_024));
/// assert_eq!(track_fragment.tfdt().unwrap().base_media_decode_time(), 0);
///
/// // Their data lies past the fragment and the header of the `mdat` beside it
/// let track_run = track_fragment.trun().first().unwrap();
/// let past_the_fragment = movie_fragment.encoded_len() + 8;
/// assert_eq!(track_run.data_offset(), Some(i32::try_from(past_the_fragment).unwrap()));
/// # Ok::<(), isobmff_sample::SampleError>(())
/// ```
#[derive(Clone, Debug)]
pub struct MovieFragmentWriter {
    decode_times: TrackDecodeTimes,
    state: State,
}

/// Where the writer stands between calls
#[derive(Clone, Debug)]
enum State {
    /// Between fragments, waiting for the next one to be opened
    Between,
    /// Laying out a fragment that was opened, and taking the samples it carries
    Fragment(OpenFragment),
    /// Failed, and reporting that same failure for every call after it
    Failed(SampleError),
}

impl MovieFragmentWriter {
    /// Creates a writer waiting for the first fragment
    #[must_use]
    pub const fn new() -> Self {
        Self {
            decode_times: TrackDecodeTimes::new(),
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
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn begin_fragment(&mut self, sequence_number: u32) -> Result<(), SampleError> {
        match self.state {
            State::Between => {}
            State::Fragment(_) => return Err(self.fail(SampleError::fragment_still_open())),
            State::Failed(failure) => return Err(failure),
        }
        self.state = State::Fragment(OpenFragment::new(sequence_number));

        Ok(())
    }

    /// Takes a sample, and places it in the fragment that is open
    ///
    /// The sample lands where it arrived: its bytes go on the end of the media
    /// data of the fragment, and what it states is written by the `traf` of
    /// its track.
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
    /// * [`SampleSizeOutOfRange`](crate::SampleErrorKind::SampleSizeOutOfRange):
    ///   the sample is longer than a `trun` row states.
    /// * [`DecodeTimeOverflow`](crate::SampleErrorKind::DecodeTimeOverflow):
    ///   the decode times of its track run past what 64 bits carry.
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), SampleError> {
        let open = match &mut self.state {
            State::Between => return Err(self.fail(SampleError::no_fragment_open())),
            State::Fragment(open) => open,
            State::Failed(failure) => return Err(*failure),
        };
        let placed = open.place(sample, &self.decode_times);

        placed.map_err(|failure| self.fail(failure))
    }

    /// Closes the fragment that is open, and hands back the `moof` and the `mdat` payload it is written as
    ///
    /// The `moof` and the payload are settled here, both held whole, so no
    /// offset is written before the length it counts from is known.
    ///
    /// # Errors
    ///
    /// * [`NoFragmentOpen`](crate::SampleErrorKind::NoFragmentOpen): no
    ///   fragment was open to close.
    /// * [`DataOffsetOutOfRange`](crate::SampleErrorKind::DataOffsetOutOfRange):
    ///   a sample lies further into the fragment than a `trun` reaches.
    /// * [`CompositionTimeOffsetOutOfRange`](crate::SampleErrorKind::CompositionTimeOffsetOutOfRange):
    ///   a sample states a composition time offset neither version of a `trun`
    ///   writes.
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn finish_fragment(&mut self) -> Result<(MovieFragmentBox, Vec<u8>), SampleError> {
        let open = match mem::replace(&mut self.state, State::Between) {
            State::Between => return Err(self.fail(SampleError::no_fragment_open())),
            State::Fragment(open) => open,
            State::Failed(failure) => {
                self.state = State::Failed(failure);

                return Err(failure);
            }
        };
        for (track_id, reached) in open.reached() {
            self.decode_times.reach(track_id, reached);
        }

        open.into_boxes().map_err(|failure| self.fail(failure))
    }

    /// Fails the writer for good, and hands the failure back to report
    fn fail(&mut self, failure: SampleError) -> SampleError {
        self.state = State::Failed(failure);

        failure
    }
}

impl Default for MovieFragmentWriter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use isobmff_boxes::MovieFragmentBox;

    use super::MovieFragmentWriter;
    use crate::error::SampleError;
    use crate::sample::Sample;

    /// Sample of `track_id` at `decode_time` lasting 1024 units, carrying `data`
    pub(super) fn sample(track_id: u32, decode_time: u64, data: &[u8]) -> Sample {
        Sample::new(track_id, decode_time, 1_024, 0, 0, 1, data.to_vec())
    }

    /// Decode time the fragment `track_id` contributed to `movie_fragment` starts at
    fn decode_time_of(movie_fragment: &MovieFragmentBox, track_id: u32) -> u64 {
        movie_fragment
            .traf()
            .iter()
            .find(|track_fragment| track_fragment.tfhd().track_id() == track_id)
            .unwrap()
            .tfdt()
            .unwrap()
            .base_media_decode_time()
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
            decode_times.push(decode_time_of(&movie_fragment, 1));
        }

        assert_eq!(decode_times, [0, 8_192]);
    }

    #[test]
    fn a_fragment_may_start_after_the_samples_before_it_end() {
        let mut writer = MovieFragmentWriter::new();

        writer.begin_fragment(1).unwrap();
        writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();
        writer.finish_fragment().unwrap();
        writer.begin_fragment(2).unwrap();

        assert_eq!(writer.handle_sample(sample(1, 8_192, b"BBBB")), Ok(()));
    }

    #[test]
    fn a_track_going_back_to_a_decode_time_it_passed_is_refused() {
        let mut writer = MovieFragmentWriter::new();

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
    fn a_sample_handed_over_or_a_fragment_closed_while_none_is_open_is_refused() {
        let mut handed_a_sample = MovieFragmentWriter::new();
        let mut closed = MovieFragmentWriter::new();

        assert_eq!(
            handed_a_sample.handle_sample(sample(1, 0, b"AAAA")),
            Err(SampleError::no_fragment_open())
        );
        assert_eq!(
            closed.finish_fragment(),
            Err(SampleError::no_fragment_open())
        );
    }

    #[test]
    fn a_fragment_begun_while_one_is_open_is_refused() {
        let mut writer = MovieFragmentWriter::new();

        writer.begin_fragment(1).unwrap();

        assert_eq!(
            writer.begin_fragment(2),
            Err(SampleError::fragment_still_open())
        );
    }

    #[test]
    fn a_failure_is_reported_again_for_every_call_after_it() {
        let mut writer = MovieFragmentWriter::new();

        writer.begin_fragment(1).unwrap();
        writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();
        writer.handle_sample(sample(1, 512, b"BBBB")).unwrap_err();

        assert_eq!(
            writer.handle_sample(sample(1, 1_024, b"CCCC")),
            Err(SampleError::decode_time_mismatch(1, 512, 1_024))
        );
        assert_eq!(
            writer.finish_fragment(),
            Err(SampleError::decode_time_mismatch(1, 512, 1_024))
        );
        assert_eq!(
            writer.begin_fragment(2),
            Err(SampleError::decode_time_mismatch(1, 512, 1_024))
        );
    }
}
