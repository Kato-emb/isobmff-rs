//! [`MovieFragmentWriter`], the samples of a presentation laid out as movie fragments, ISO/IEC 14496-12 §8.8

mod open_fragment;

use alloc::vec::Vec;
use core::mem;

use isobmff_boxes::{MovieBox, MovieFragmentBox, TrackBox, TrackExtendsBox};

use crate::error::Error;
use crate::movie_fragment_writer::open_fragment::OpenFragment;
use crate::sample::Sample;
use crate::track_decode_times::TrackDecodeTimes;

/// Lays the samples of a presentation out as movie fragments
///
/// The writer takes the samples of a fragment between
/// [`begin_fragment`](Self::begin_fragment) or
/// [`begin_fragment_continuing`](Self::begin_fragment_continuing) and
/// [`finish_fragment`](Self::finish_fragment), which hands back the `moof`
/// they are declared by and the payload of the `mdat` that carries them. It
/// writes nothing itself: what the two are laid down as, and where, stay with
/// the caller.
///
/// The writer is made for the movie the fragments continue, which it checks
/// the samples against and never writes: the brands and the movie stay the
/// caller's too. The `trex` of a track sets defaults this writer never leans
/// on — every default a fragment falls back on is stated by its own `tfhd`.
///
/// # Layout
///
/// Samples are laid out in the order they arrive.
///
/// * The media data holds the samples in the order they were handed over. A
///   caller interleaving two tracks states that by handing them over
///   interleaved.
/// * One `traf` per track, in the order the tracks first appear. A run of
///   samples of one track handed over together is one `trun`.
/// * A fragment therefore declares its samples track by track, whatever order
///   they were handed over in — the media data holds that order, the `traf`
///   boxes group it.
/// * Offsets are anchored at the fragment itself
///   ([`default-base-is-moof`](isobmff_boxes::TrackFragmentHeaderFlags::DEFAULT_BASE_IS_MOOF),
///   ISO/IEC 14496-12 §8.8.7.1), and every `trun` states its own. They are
///   counted over the `moof` and the header of the `mdat`, so the two are laid
///   down as they come out: the media data of a fragment directly after the
///   fragment itself.
/// * A `tfdt` is always written, stating the decode time the fragment places
///   the first sample of its `traf` at.
/// * What the samples of a `traf` share — how long they last, how long they
///   are, their flags — is written once as a default of its `tfhd`, and left
///   out of the rows. What they do not share is stated per row, except flags
///   that only the first sample differs on, which are its
///   `first_sample_flags`. The `stsd` entry is always stated by the `tfhd`.
/// * A run whose samples all compose when they are decoded states no
///   composition time offset; one holding any other offset states one for
///   every row.
///
/// # Contract
///
/// * The movie is one continued in fragments: it carries an `mvex`, which is
///   otherwise [`MissingMovieExtends`](crate::ErrorKind::MissingMovieExtends),
///   and its sample tables lay no sample out, which is otherwise
///   [`SampleTableNotEmpty`](crate::ErrorKind::SampleTableNotEmpty) — both
///   reported by [`new`](Self::new).
/// * A sample belongs to a track the movie declares by a `trak` and a `trex`,
///   and is described by an `stsd` entry of that track whose data reference
///   is the file itself, as a reader of the fragments resolves it (§8.8.3,
///   §8.5.2, §8.7.2). The first sample of a track in a fragment is checked,
///   and the samples after it are held to its entry: the failures are those
///   of the reader —
///   [`UnknownTrackId`](crate::ErrorKind::UnknownTrackId),
///   [`UnknownSampleDescriptionIndex`](crate::ErrorKind::UnknownSampleDescriptionIndex),
///   [`UnknownDataReferenceIndex`](crate::ErrorKind::UnknownDataReferenceIndex),
///   [`ExternalDataReference`](crate::ErrorKind::ExternalDataReference), or
///   the failure of an entry that does not read as a sample entry, carried
///   on [`Box`](crate::ErrorKind::Box).
/// * A fragment is opened by [`begin_fragment`](Self::begin_fragment) or
///   [`begin_fragment_continuing`](Self::begin_fragment_continuing) and
///   closed by [`finish_fragment`](Self::finish_fragment). Handing a sample
///   over or closing a fragment while none is open is
///   [`NoFragmentOpen`](crate::ErrorKind::NoFragmentOpen), and opening
///   one while one is open is
///   [`FragmentStillOpen`](crate::ErrorKind::FragmentStillOpen).
/// * The `sequence_number` of a fragment is the caller's. §8.8.5 has it
///   increase over the fragments of a presentation, which the writer neither
///   checks nor reports.
/// * Within one fragment, a sample of a track starts where the one before it
///   ends: a `trun` states how long a sample lasts and not when it is decoded,
///   so a gap is
///   [`DecodeTimeMismatch`](crate::ErrorKind::DecodeTimeMismatch).
///   Between fragments opened by [`begin_fragment`](Self::begin_fragment) a
///   gap is written as it stands — the `tfdt` states it —
///   but a track never goes back, which is
///   [`BackwardDecodeTime`](crate::ErrorKind::BackwardDecodeTime).
/// * A fragment opened by
///   [`begin_fragment`](Self::begin_fragment) places a track where its first
///   sample states. One opened by
///   [`begin_fragment_continuing`](Self::begin_fragment_continuing) places it
///   where the samples written for it reach, zero for a track none was
///   written for, whatever its first sample states: the samples after it are
///   read only for how far they lie from that first one, which the fragment
///   keeps.
/// * The samples of one `traf` are all described by one `stsd` entry, which
///   the `tfhd` states for them: a fragment mixing two is
///   [`SampleDescriptionIndexMismatch`](crate::ErrorKind::SampleDescriptionIndexMismatch).
/// * A run stating a negative composition time offset and one past
///   [`i32::MAX`], which no one version of a `trun` writes both of (§8.8.8),
///   is reported by [`finish_fragment`](Self::finish_fragment), where the
///   runs are built:
///   [`CompositionTimeOffsetOutOfRange`](crate::ErrorKind::CompositionTimeOffsetOutOfRange).
/// * A fragment of no samples is written as a `moof` of no `traf` beside an
///   empty payload.
/// * An `Err` leaves the writer failed for good,
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished) aside: every
///   later call reports that same failure again.
/// * [`finish`](Self::finish) declares the samples over, and fails if a
///   fragment is still open. A fragment opened or closed, or a sample handed
///   over then, or a second [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished).
///
/// An empty `traf` stating a `tfdt` alone, which §8.8.12 allows for
/// establishing the duration of the sample before it, is not written: a track
/// reaches a fragment only by a sample of it.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{SampleFlags, TrackExtendsBox};
/// use isobmff_core::BoxEncode as _;
/// use isobmff_sample::{MovieFragmentWriter, Sample};
/// # use isobmff_test_support::fragmented_movie;
///
/// // A writer for a movie of track 1, continued in fragments
/// let movie = fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO));
/// let mut writer = MovieFragmentWriter::new(&movie)?;
///
/// // One fragment of two samples of track 1, lasting 1024 units each
/// writer.begin_fragment(1)?;
/// writer.handle_sample(Sample::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec()))?;
/// writer.handle_sample(Sample::new(1, 1_024, 1_024, 0, SampleFlags::ZERO, 1, b"DATA".to_vec()))?;
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
/// writer.finish()?;
/// # Ok::<(), isobmff_sample::Error>(())
/// ```
#[derive(Clone, Debug)]
pub struct MovieFragmentWriter {
    trak: Vec<TrackBox>,
    trex: Vec<TrackExtendsBox>,
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
    /// Told the samples are over, and taking nothing more
    Finished,
    /// Failed, and reporting that same failure for every call after it
    Failed(Error),
}

impl MovieFragmentWriter {
    /// Creates a writer for the fragments `movie` continues, waiting for the first one
    ///
    /// # Errors
    ///
    /// * [`MissingMovieExtends`](crate::ErrorKind::MissingMovieExtends):
    ///   the movie carries no `mvex`, and so continues in no fragments.
    /// * [`SampleTableNotEmpty`](crate::ErrorKind::SampleTableNotEmpty):
    ///   the sample tables of a track lay samples out.
    pub fn new(movie: &MovieBox) -> Result<Self, Error> {
        let Some(mvex) = movie.mvex() else {
            return Err(Error::missing_movie_extends());
        };
        for trak in movie.trak() {
            let stbl = trak.mdia().minf().stbl();
            if !stbl.stts().entries().is_empty()
                || !stbl.stsc().entries().is_empty()
                || stbl.sample_sizes().sizes().next().is_some()
                || stbl.chunk_offsets().offsets().next().is_some()
            {
                return Err(Error::sample_table_not_empty(trak.tkhd().track_id()));
            }
        }

        Ok(Self {
            trak: movie.trak().to_vec(),
            trex: mvex.trex().to_vec(),
            decode_times: TrackDecodeTimes::new(movie)?,
            state: State::Between,
        })
    }

    /// Opens a fragment, which the samples handed over next are laid out in
    ///
    /// `sequence_number` is what its `mfhd` states. §8.8.5 has it increase over
    /// the fragments of a presentation, which is the caller's to hold to.
    ///
    /// # Errors
    ///
    /// * [`FragmentStillOpen`](crate::ErrorKind::FragmentStillOpen): the
    ///   fragment before it was not closed.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   samples were declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn begin_fragment(&mut self, sequence_number: u32) -> Result<(), Error> {
        self.open_fragment(sequence_number, TrackDecodeTimes::unknown())
    }

    /// Opens a fragment in which every track continues where the samples written for it reach
    ///
    /// The first sample of a track the fragment carries is placed where the
    /// samples written for the track reach — zero for a track none was
    /// written for — whatever decode time it states, and every later one by
    /// how far its stated decode time lies from that of the first. The `tfdt`
    /// of the track states that reached time, and the durations and the
    /// composition time offsets of the samples are written as handed over.
    /// `sequence_number` is what its `mfhd` states, as for
    /// [`begin_fragment`](Self::begin_fragment).
    ///
    /// # Errors
    ///
    /// * [`FragmentStillOpen`](crate::ErrorKind::FragmentStillOpen): the
    ///   fragment before it was not closed.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   samples were declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn begin_fragment_continuing(&mut self, sequence_number: u32) -> Result<(), Error> {
        self.open_fragment(sequence_number, self.decode_times.clone())
    }

    /// Takes a sample, and places it in the fragment that is open
    ///
    /// The sample lands where it arrived: its bytes go on the end of the media
    /// data of the fragment, and what it states is written by the `traf` of
    /// its track.
    ///
    /// # Errors
    ///
    /// * [`NoFragmentOpen`](crate::ErrorKind::NoFragmentOpen): no
    ///   fragment was opened to carry it.
    /// * [`UnknownTrackId`](crate::ErrorKind::UnknownTrackId): the movie
    ///   declares no `trak` or no `trex` for the track of the sample.
    /// * [`UnknownSampleDescriptionIndex`](crate::ErrorKind::UnknownSampleDescriptionIndex):
    ///   the track has no `stsd` entry describing the sample.
    /// * The failures of [`SampleEntry::try_from`](isobmff_boxes::SampleEntry),
    ///   carried on [`Box`](crate::ErrorKind::Box): that entry does not read
    ///   as a sample entry, with `stsd` added to the containers.
    /// * [`UnknownDataReferenceIndex`](crate::ErrorKind::UnknownDataReferenceIndex):
    ///   that entry names a `dref` entry the track has none of.
    /// * [`ExternalDataReference`](crate::ErrorKind::ExternalDataReference):
    ///   the `dref` entry names a resource other than the file itself.
    /// * [`DecodeTimeMismatch`](crate::ErrorKind::DecodeTimeMismatch):
    ///   the sample does not start where the one before it in its track ends.
    /// * [`BackwardDecodeTime`](crate::ErrorKind::BackwardDecodeTime):
    ///   the sample starts before the samples written for its track reach.
    /// * [`SampleDescriptionIndexMismatch`](crate::ErrorKind::SampleDescriptionIndexMismatch):
    ///   the sample is described by another `stsd` entry than its fragment
    ///   states for the track.
    /// * [`SampleSizeOutOfRange`](crate::ErrorKind::SampleSizeOutOfRange):
    ///   the sample is longer than the 32 bits a `trun` row states its length
    ///   in.
    /// * [`CompositionTimeOffsetOutOfRange`](crate::ErrorKind::CompositionTimeOffsetOutOfRange):
    ///   the sample states a composition time offset neither version of a
    ///   `trun` writes.
    /// * [`DecodeTimeOverflow`](crate::ErrorKind::DecodeTimeOverflow):
    ///   the decode times of its track run past what 64 bits carry.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   samples were declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), Error> {
        self.writing()?;
        let State::Fragment(open) = &mut self.state else {
            return Err(self.fail(Error::no_fragment_open()));
        };
        open.place(sample, &self.trak, &self.trex, &self.decode_times)
            .map_err(|failure| self.fail(failure))
    }

    /// Closes the fragment that is open, and hands back the `moof` and the `mdat` payload it is written as
    ///
    /// The offsets count over the header
    /// [`MediaDataBox`](isobmff_boxes::MediaDataBox) writes for a payload of
    /// that length, so the payload is laid down as that box, directly after
    /// the `moof`. Where the samples leave the timeline of each track is kept
    /// once the pair is built, and not on a failure.
    ///
    /// # Errors
    ///
    /// * [`NoFragmentOpen`](crate::ErrorKind::NoFragmentOpen): no
    ///   fragment was open to close.
    /// * [`DataOffsetOutOfRange`](crate::ErrorKind::DataOffsetOutOfRange):
    ///   a sample lies further into the fragment than a `trun` reaches.
    /// * [`CompositionTimeOffsetOutOfRange`](crate::ErrorKind::CompositionTimeOffsetOutOfRange):
    ///   a run states a negative composition time offset and one past
    ///   [`i32::MAX`], which no one version of a `trun` writes both of; the
    ///   failure names the widest.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   samples were declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn finish_fragment(&mut self) -> Result<(MovieFragmentBox, Vec<u8>), Error> {
        self.writing()?;
        // Why not leaving the state alone until the fragment is known to build:
        // the boxes are built from the fragment whole, and the caller reached
        // here through `writing`, so the only state this replaces without a
        // fragment to take is the `Between` it puts back.
        let State::Fragment(open) = mem::replace(&mut self.state, State::Between) else {
            return Err(self.fail(Error::no_fragment_open()));
        };

        open.into_boxes(&mut self.decode_times)
            .map_err(|failure| self.fail(failure))
    }

    /// Declares the samples over
    ///
    /// # Errors
    ///
    /// * [`FragmentStillOpen`](crate::ErrorKind::FragmentStillOpen): a
    ///   fragment was left open.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   samples were already declared over.
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.writing()?;
        if matches!(self.state, State::Fragment(_)) {
            return Err(self.fail(Error::fragment_still_open()));
        }
        self.state = State::Finished;

        Ok(())
    }

    /// Opens a fragment numbered `sequence_number` whose tracks `placement` places
    fn open_fragment(
        &mut self,
        sequence_number: u32,
        placement: TrackDecodeTimes,
    ) -> Result<(), Error> {
        self.writing()?;
        if matches!(self.state, State::Fragment(_)) {
            return Err(self.fail(Error::fragment_still_open()));
        }
        self.state = State::Fragment(OpenFragment::new(sequence_number, placement));

        Ok(())
    }

    /// Returns `Ok` while the writer still takes samples
    const fn writing(&self) -> Result<(), Error> {
        match self.state {
            State::Between | State::Fragment(_) => Ok(()),
            State::Finished => Err(Error::already_finished()),
            State::Failed(failure) => Err(failure),
        }
    }

    /// Fails the writer for good, and hands the failure back to report
    fn fail(&mut self, failure: Error) -> Error {
        self.state = State::Failed(failure);

        failure
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_boxes::{
        ChunkOffsetBox, ChunkOffsetEntry, ChunkOffsets, MovieBox, SampleFlags, SampleSizeBox,
        SampleSizeEntries, SampleSizeEntry, SampleSizes, SampleToChunkBox, SampleToChunkEntry,
        TimeToSampleBox, TimeToSampleEntry,
    };
    use isobmff_test_support::{
        external_data_reference, sample_table, self_contained_data_reference, track,
        track_laid_out, track_reading_from, unfragmented_movie,
    };

    use super::MovieFragmentWriter;
    use crate::error::Error;
    use crate::sample::Sample;

    /// Writer for a movie of tracks 1 and 2, each continued in fragments
    pub(super) fn writer() -> MovieFragmentWriter {
        let movie = MovieBox::new_fragmented(90_000, vec![track(1), track(2)]).unwrap();

        MovieFragmentWriter::new(&movie).unwrap()
    }

    /// Sample of `track_id` at `decode_time` lasting 1024 units, carrying `data`
    pub(super) fn sample(track_id: u32, decode_time: u64, data: &[u8]) -> Sample {
        Sample::new(
            track_id,
            decode_time,
            1_024,
            0,
            SampleFlags::ZERO,
            1,
            data.to_vec(),
        )
    }

    #[test]
    fn a_fragment_may_start_after_the_samples_before_it_end() {
        let mut writer = writer();

        writer.begin_fragment(1).unwrap();
        writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();
        writer.finish_fragment().unwrap();
        writer.begin_fragment(2).unwrap();

        assert_eq!(writer.handle_sample(sample(1, 8_192, b"BBBB")), Ok(()));
    }

    #[test]
    fn a_track_going_back_to_a_decode_time_it_passed_is_refused() {
        let mut writer = writer();

        writer.begin_fragment(1).unwrap();
        writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();
        writer.finish_fragment().unwrap();
        writer.begin_fragment(2).unwrap();

        assert_eq!(
            writer.handle_sample(sample(1, 512, b"BBBB")),
            Err(Error::backward_decode_time(1, 512, 1_024))
        );
    }

    #[test]
    fn a_fragment_opened_after_one_opened_continuing_carries_on_from_where_it_placed_the_track() {
        let mut writer = writer();

        writer.begin_fragment(1).unwrap();
        writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();
        writer.finish_fragment().unwrap();
        writer.begin_fragment_continuing(2).unwrap();
        writer.handle_sample(sample(1, 90_000, b"BBBB")).unwrap();
        writer.finish_fragment().unwrap();
        let mut going_back = writer.clone();
        writer.begin_fragment(3).unwrap();
        going_back.begin_fragment(3).unwrap();

        assert_eq!(writer.handle_sample(sample(1, 2_048, b"CCCC")), Ok(()));
        assert_eq!(
            going_back.handle_sample(sample(1, 2_047, b"CCCC")),
            Err(Error::backward_decode_time(1, 2_047, 2_048))
        );
    }

    #[test]
    fn a_sample_handed_over_or_a_fragment_closed_while_none_is_open_is_refused() {
        let mut handed_a_sample = writer();
        let mut closed = writer();

        assert_eq!(
            handed_a_sample.handle_sample(sample(1, 0, b"AAAA")),
            Err(Error::no_fragment_open())
        );
        assert_eq!(closed.finish_fragment(), Err(Error::no_fragment_open()));
    }

    #[test]
    fn the_samples_are_declared_over_once_every_fragment_is_closed() {
        let mut writer = writer();

        writer.begin_fragment(1).unwrap();
        writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();
        writer.finish_fragment().unwrap();

        assert_eq!(writer.finish(), Ok(()));
    }

    #[test]
    fn anything_handed_over_after_the_samples_were_declared_over_is_refused() {
        let mut writer = writer();

        writer.finish().unwrap();

        assert_eq!(writer.begin_fragment(1), Err(Error::already_finished()));
        assert_eq!(
            writer.handle_sample(sample(1, 0, b"AAAA")),
            Err(Error::already_finished())
        );
        assert_eq!(writer.finish_fragment(), Err(Error::already_finished()));
        assert_eq!(writer.finish(), Err(Error::already_finished()));
    }

    #[test]
    fn the_samples_declared_over_while_a_fragment_is_open_is_refused() {
        let mut writer = writer();

        writer.begin_fragment(1).unwrap();

        assert_eq!(writer.finish(), Err(Error::fragment_still_open()));
    }

    #[test]
    fn a_fragment_begun_while_one_is_open_is_refused() {
        let mut writer = writer();

        writer.begin_fragment(1).unwrap();

        assert_eq!(writer.begin_fragment(2), Err(Error::fragment_still_open()));
    }

    #[test]
    fn a_failure_is_reported_again_for_every_call_after_it() {
        let mut writer = writer();

        writer.begin_fragment(1).unwrap();
        writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();
        writer.handle_sample(sample(1, 512, b"BBBB")).unwrap_err();

        assert_eq!(
            writer.handle_sample(sample(1, 1_024, b"CCCC")),
            Err(Error::decode_time_mismatch(1, 512, 1_024))
        );
        assert_eq!(
            writer.finish_fragment(),
            Err(Error::decode_time_mismatch(1, 512, 1_024))
        );
        assert_eq!(
            writer.begin_fragment(2),
            Err(Error::decode_time_mismatch(1, 512, 1_024))
        );
        assert_eq!(
            writer.finish(),
            Err(Error::decode_time_mismatch(1, 512, 1_024))
        );
    }

    #[test]
    fn a_writer_is_made_only_for_a_movie_continued_in_fragments() {
        let laid_out = sample_table(
            TimeToSampleBox::new(vec![TimeToSampleEntry::new(1, 1_024)]),
            SampleToChunkBox::new(vec![SampleToChunkEntry::new(1, 1, 1)]),
            SampleSizes::Stsz(SampleSizeBox::new(SampleSizeEntries::PerSample(vec![
                SampleSizeEntry::new(4),
            ]))),
            ChunkOffsets::Stco(ChunkOffsetBox::new(vec![ChunkOffsetEntry::new(8)])),
        );
        let filled = MovieBox::new_fragmented(
            90_000,
            vec![
                track(1),
                track_laid_out(2, self_contained_data_reference(), laid_out),
            ],
        )
        .unwrap();

        assert_eq!(
            MovieFragmentWriter::new(&unfragmented_movie()).map(|_writer| ()),
            Err(Error::missing_movie_extends())
        );
        assert_eq!(
            MovieFragmentWriter::new(&filled).map(|_writer| ()),
            Err(Error::sample_table_not_empty(2))
        );
    }

    #[test]
    fn a_sample_the_movie_does_not_resolve_is_refused_where_it_is_handed_over() {
        let movie = MovieBox::new_fragmented(
            90_000,
            vec![track(1), track_reading_from(2, external_data_reference())],
        )
        .unwrap();
        let refused = |sample: Sample| {
            let mut writer = MovieFragmentWriter::new(&movie).unwrap();
            writer.begin_fragment(1).unwrap();

            writer.handle_sample(sample)
        };

        assert_eq!(
            refused(sample(999, 0, b"AAAA")),
            Err(Error::unknown_track_id(999))
        );
        assert_eq!(
            refused(Sample::new(
                1,
                0,
                1_024,
                0,
                SampleFlags::ZERO,
                2,
                b"AAAA".to_vec()
            )),
            Err(Error::unknown_sample_description_index(1, 2))
        );
        assert_eq!(
            refused(sample(2, 0, b"AAAA")),
            Err(Error::external_data_reference(2, 1))
        );
    }

    #[test]
    fn a_sample_of_a_track_no_trex_continues_in_fragments_is_refused() {
        let mut movie = MovieBox::new_fragmented(90_000, vec![track(1)]).unwrap();
        *movie.trak_mut(1).unwrap() = track(5);
        let mut writer = MovieFragmentWriter::new(&movie).unwrap();

        writer.begin_fragment(1).unwrap();

        assert_eq!(
            writer.handle_sample(sample(5, 0, b"AAAA")),
            Err(Error::unknown_track_id(5))
        );
    }

    #[test]
    fn a_fragment_holding_a_refused_sample_is_never_built() {
        let mut writer = writer();

        writer.begin_fragment(1).unwrap();
        writer.handle_sample(sample(999, 0, b"AAAA")).unwrap_err();

        assert_eq!(writer.finish_fragment(), Err(Error::unknown_track_id(999)));
    }
}
