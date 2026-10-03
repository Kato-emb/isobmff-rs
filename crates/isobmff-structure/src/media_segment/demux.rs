//! [`MediaSegmentDemuxFsm`], a media segment read as it arrives

use alloc::vec::Vec;

use isobmff_boxes::{MovieBox, MovieFragmentBox, SegmentIndexBox, SegmentTypeBox};
use isobmff_sample::movie_fragment::sample_extents;
use isobmff_sample::segment_index::subsegments;
use isobmff_sample::{Sample, SampleReader, SegmentIndex, TrackDecodeTimes};
use isobmff_sequence::{BoxEvent, BoxReader};

use super::{MediaSegmentDisposition, MediaSegmentStructure};
use crate::{Error, InputPosition, InputRoute, WantedInput, WholeBoxReader};

/// Reads the samples a media segment carries, taking it as it arrives
///
/// A media segment carries a portion of a presentation for delivery apart
/// from the movie that declares it (ISO/IEC 14496-12 §8.16.1): the brands it
/// declares itself readable as, then one movie fragment after another with
/// the media data each of them addresses. This reader wires the layers that
/// read one: the framing of the segment into boxes, the structure that says
/// what each top-level box is, the reading of the boxes it names into values,
/// the resolution of each fragment against the movie into the extents of its
/// samples, and the gathering of those samples out of the media data. The
/// movie is the caller's to hand over, since the segment carries none. It
/// holds no rule of its own but one: of the bytes the samples still lack, it
/// names only those whose start the segment handed over in order has passed.
/// A caller hands over bytes and takes [`Sample`]s. It reaches for no source
/// of its own: when to read and from where stay with the caller.
///
/// # Contract
///
/// * The segment is handed over from its first byte, or from the offset
///   [`resume_at`](Self::resume_at) names, in order and cut anywhere,
///   each cut handed over at the offset it was read at, where
///   [`wanted_input`](Self::wanted_input) names,
///   and the samples it completed are taken from
///   [`poll_sample`](Self::poll_sample). The caller drains before handing
///   over more: samples are held until they are taken. Where the segment
///   lies in its resource is the caller's: every offset the reader reports
///   is an offset into the segment, counting from the first byte of it as
///   the boxes count theirs (§8.8.7).
/// * The fragments are resolved against the movie the reader was created
///   with, which is there to read at [`movie`](Self::movie). The brands are
///   there once they have arrived: [`segment_type`](Self::segment_type). The
///   indexes of every `sidx` are there once read, as
///   [`segment_indexes`](Self::segment_indexes). The media data is offered
///   to the samples, and every other box is passed over — a `moov` among
///   them.
/// * [`resume_at`](Self::resume_at) restarts the reading at an offset into
///   the segment an index points at, a `moof` or a `sidx`, from where the
///   segment is then handed over. The indexes read so far stand; the extents
///   held and the samples not yet taken are dropped, and where each track
///   stands on its timeline is no longer known until a `tfdt` states it; a
///   fragment stating none for such a track is
///   [`Sample`](crate::ErrorKind::Sample).
/// * The order the boxes come in, and what a segment that breaks it is
///   reported as, are the structure's: a `styp` after another box and an
///   `mdat` before any `moof` are
///   [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder), and a
///   segment declared over without a `moof` is
///   [`MissingMandatoryBox`](crate::ErrorKind::MissingMandatoryBox).
///   A segment carrying no `styp` reads all the same, as §8.16.2 allows.
/// * Where a fragment states no decode time for a track, the track goes on
///   from where the fragments handed over before it left it, or from zero
///   where none did (§8.8.12): a reader is one segment's. After a
///   [`resume_at`](Self::resume_at) it is [`Sample`](crate::ErrorKind::Sample)
///   instead.
/// * A box read into a value is gathered whole before it is read, so what it
///   declares is bounded — see [`with_limits`](Self::with_limits).
/// * The samples of a fragment are read out of the media data that follows
///   it, and come out as their bytes arrive whole, as [`SampleReader`]'s
///   contract has it: the extents of a fragment are held in the order of
///   their bytes, so a segment handed over in order yields the samples of each
///   fragment in the order they lie in it, whatever order the fragment
///   declares them in and wherever the input is cut.
///   [`wanted_input`](Self::wanted_input) names what the extent at the
///   front of those held still lacks only once the input has passed its
///   start, which a segment whose fragments precede their media data never has.
/// * An `Err` leaves the reader failed for good,
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished) and
///   [`UnwantedInput`](crate::ErrorKind::UnwantedInput) aside:
///   every later call reports that same failure again. The samples completed
///   before it are still there to take.
/// * [`finish`](Self::finish) declares the segment over, and reports what
///   any layer makes of the end of it: a box left open, no `moof` come, a
///   sample short of the data it claimed. Samples are still taken after it,
///   but anything handed over then, or a second [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished), until
///   [`resume_at`](Self::resume_at) restarts the reading.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{SampleFlags, TrackExtendsBox};
/// use isobmff_sample::Sample;
/// use isobmff_structure::{MediaSegmentDemuxFsm, MediaSegmentMuxFsm};
/// # use isobmff_test_support::{fragmented_movie, segment_type};
/// // A segment of one fragment carrying two samples of track 1
/// let mut mux_fsm = MediaSegmentMuxFsm::new();
/// mux_fsm.handle_segment_type(segment_type())?;
/// mux_fsm.begin_fragment(1)?;
/// mux_fsm.handle_sample(Sample::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec()))?;
/// mux_fsm.handle_sample(Sample::new(1, 1_024, 1_024, 0, SampleFlags::ZERO, 1, b"DATA".to_vec()))?;
/// mux_fsm.finish_fragment()?;
/// mux_fsm.finish()?;
///
/// // The segment the mux FSM laid down is drained as it hands the bytes over
/// let mut segment = Vec::new();
/// while let Some(written) = mux_fsm.poll_output() {
///     segment.extend_from_slice(&written);
/// }
///
/// // The segment is handed over as it arrives, against the movie it continues
/// let movie = fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO));
/// let mut demux_fsm = MediaSegmentDemuxFsm::new(movie);
/// for (offset, arriving) in (0..).step_by(7).zip(segment.chunks(7)) {
///     demux_fsm.handle_input(offset, arriving)?;
/// }
/// demux_fsm.finish()?;
///
/// // The brands the segment declared are there to read
/// assert_eq!(demux_fsm.segment_type().map(|styp| styp.major_brand()), Some(segment_type().major_brand()));
///
/// // The samples come back as they were laid out
/// let first = demux_fsm.poll_sample().unwrap();
/// assert_eq!((first.data(), first.decode_time()), (b"SAMP".as_slice(), 0));
/// let second = demux_fsm.poll_sample().unwrap();
/// assert_eq!((second.data(), second.decode_time()), (b"DATA".as_slice(), 1_024));
/// assert_eq!(demux_fsm.poll_sample(), None);
/// # Ok::<(), isobmff_structure::Error>(())
/// ```
#[derive(Debug)]
pub struct MediaSegmentDemuxFsm {
    boxes: BoxReader,
    position: InputPosition,
    structure: MediaSegmentStructure,
    samples: SampleReader,
    decode_times: TrackDecodeTimes,
    open: Option<Open>,
    segment_type: Option<SegmentTypeBox>,
    movie: MovieBox,
    segment_indexes: Vec<SegmentIndex>,
    payload_limit: u64,
    state: State,
}

/// Where the reader stands between calls
#[derive(Clone, Copy, Debug)]
enum State {
    /// Taking the segment as it arrives
    Reading,
    /// Told the segment is over, and taking no more input
    Finished,
    /// Failed, and reporting that same failure for every call after it
    Failed(Error),
}

/// The top-level box that started, held as its disposition has it until it ends
#[derive(Debug)]
enum Open {
    /// Brands being read whole
    SegmentType(WholeBoxReader<SegmentTypeBox>),
    /// Fragment being read whole, and where in the segment it began
    MovieFragment {
        reader: WholeBoxReader<MovieFragmentBox>,
        moof_start: u64,
    },
    /// Segment index being read whole
    SegmentIndex(WholeBoxReader<SegmentIndexBox>),
    /// Media data, offered to the samples as it arrives
    MediaData,
}

impl MediaSegmentDemuxFsm {
    /// Payload a box read into a value may declare, where the caller names no limit
    ///
    /// Sixteen mebibytes. A caller reading segments whose `moof` reaches past
    /// that names a limit of its own with [`with_limits`](Self::with_limits).
    pub const DEFAULT_PAYLOAD_LIMIT: u64 = 16 * 1024 * 1024;

    /// Creates a reader waiting at the start of a media segment continuing `movie`
    ///
    /// What a box read into a value may declare is bounded by
    /// [`DEFAULT_PAYLOAD_LIMIT`](Self::DEFAULT_PAYLOAD_LIMIT), and what one
    /// sample may declare by
    /// [`SampleReader::DEFAULT_SAMPLE_SIZE_LIMIT`](SampleReader::DEFAULT_SAMPLE_SIZE_LIMIT).
    #[must_use]
    pub const fn new(movie: MovieBox) -> Self {
        Self::with_limits(
            movie,
            Self::DEFAULT_PAYLOAD_LIMIT,
            SampleReader::DEFAULT_SAMPLE_SIZE_LIMIT,
        )
    }

    /// Creates a reader of a segment continuing `movie`, holding it to `payload_limit` and `sample_size_limit`
    ///
    /// Both bound memory the reader is about to take, and both bound one box or
    /// one sample rather than the segment. A box read into a value that
    /// declares more than `payload_limit` bytes of payload is
    /// [`PayloadLimitExceeded`](crate::ErrorKind::PayloadLimitExceeded)
    /// before a byte of it is gathered; a sample declaring more than
    /// `sample_size_limit` bytes is what
    /// [`SampleReader::with_sample_size_limit`](SampleReader::with_sample_size_limit)
    /// makes of it.
    #[must_use]
    pub const fn with_limits(movie: MovieBox, payload_limit: u64, sample_size_limit: u64) -> Self {
        Self {
            boxes: BoxReader::new(),
            position: InputPosition::new(),
            structure: MediaSegmentStructure::new(),
            samples: SampleReader::with_sample_size_limit(sample_size_limit),
            decode_times: TrackDecodeTimes::new(),
            open: None,
            segment_type: None,
            movie,
            segment_indexes: Vec::new(),
            payload_limit,
            state: State::Reading,
        }
    }

    /// Takes bytes of the segment read at `offset`, and reads the samples they complete
    ///
    /// `offset` is an offset into the segment, counted from its first byte as a
    /// base data offset is (ISO/IEC 14496-12 §8.8.7). Bytes at the offset the
    /// input taken in order stands at — the first byte of the segment or the
    /// offset the last [`resume_at`](Self::resume_at) named, then where the
    /// bytes taken in order before them end — are taken whole as its
    /// continuation. Bytes at the offset of the bytes
    /// [`wanted_input`](Self::wanted_input) names as lacking are offered to the
    /// samples alone, as media data is, and the input taken in order goes on
    /// from where it stood; bytes at the offset the input taken in order stands
    /// at are taken all the same while such bytes are named. Empty input is
    /// taken as nothing. What the input completed is then taken from
    /// [`poll_sample`](Self::poll_sample).
    ///
    /// # Errors
    ///
    /// * [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder): what
    ///   the structure makes of a top-level box arriving where it does.
    /// * [`PayloadLimitExceeded`](crate::ErrorKind::PayloadLimitExceeded):
    ///   a box read into a value reaches past the limit the reader gathers.
    /// * [`Sequence`](crate::ErrorKind::Sequence): what the framing
    ///   of the segment makes of the input.
    /// * [`Box`](crate::ErrorKind::Box): a box read into a value
    ///   does not decode.
    /// * [`Sample`](crate::ErrorKind::Sample): what the samples make
    ///   of a fragment, a `sidx`, or the media data beside it.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   segment was declared over by [`finish`](Self::finish).
    /// * [`UnwantedInput`](crate::ErrorKind::UnwantedInput): `offset` is
    ///   neither where the input taken in order stands nor the offset of the
    ///   bytes [`wanted_input`](Self::wanted_input) names as lacking. The
    ///   reader is not failed by it.
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn handle_input(&mut self, offset: u64, input: &[u8]) -> Result<(), Error> {
        self.reading()?;
        if input.is_empty() {
            return Ok(());
        }

        match self.position.route(offset, self.samples.wanted_extent()) {
            InputRoute::InOrder => {}
            InputRoute::Lacking => {
                return self
                    .samples
                    .handle_data(offset, input)
                    .map_err(|failure| self.fail(failure.into()));
            }
            InputRoute::Unwanted => return Err(Error::unwanted_input(offset)),
        }

        self.position.advance(input.len());

        // Why not failing before the events are read: the framing keeps the
        // events it made before failing, and the samples they complete are
        // the caller's to take, so they are read first and the failure kept
        // for after them.
        let framed = self.boxes.handle_input(input);
        self.read_framed()?;

        framed.map_err(|failure| self.fail(failure.into()))
    }

    /// Takes the next sample the segment handed over so far completed
    ///
    /// Reports `None` once they are used up: more of the segment is needed.
    /// Failure is reported by the calls that take it, so this one never fails
    /// — a failed reader hands over the samples it had already completed, then
    /// `None` from there on.
    pub fn poll_sample(&mut self) -> Option<Sample> {
        self.samples.poll_sample()
    }

    /// Returns the one read wanted next, or `None` once the segment is declared over or the reader has failed
    ///
    /// Bytes the extent at the front of those held still lacks are wanted
    /// first, with their length, once the input taken in order has passed their
    /// start. Where each fragment precedes the media data it addresses, a
    /// segment handed over in order has none wanted; a fragment addressing
    /// media data lying before it (§8.8.7 has a base data offset name any byte
    /// of the segment) wants bytes already passed by. Otherwise the
    /// continuation of the input taken in order is wanted, at the offset it
    /// stands at and with no length. Bytes read for either are handed to
    /// [`handle_input`](Self::handle_input) at the offset they were read at.
    #[must_use]
    pub fn wanted_input(&self) -> Option<WantedInput> {
        matches!(self.state, State::Reading)
            .then(|| self.position.wanted_input(self.samples.wanted_extent()))
    }

    /// Returns the brands the segment declares itself readable as, once they have arrived
    #[must_use]
    pub const fn segment_type(&self) -> Option<&SegmentTypeBox> {
        self.segment_type.as_ref()
    }

    /// Returns the movie the fragments of the segment continue
    #[must_use]
    pub const fn movie(&self) -> &MovieBox {
        &self.movie
    }

    /// Returns the subsegments of every `sidx` read so far, in the order they were read
    ///
    /// A `sidx` read again, as the reading resumes at or before it, is held
    /// once. Each is placed in the segment from the first byte after its `sidx`, as
    /// [`subsegments`] places them.
    #[must_use]
    pub fn segment_indexes(&self) -> &[SegmentIndex] {
        &self.segment_indexes
    }

    /// Restarts the reading at `offset`, the offset into the segment the input handed over next starts at
    ///
    /// The next box is to be one an index points at: a `moof` or a `sidx`.
    /// The reader resumes from reading and from the segment declared over
    /// alike, and takes the segment again from there.
    ///
    /// # Errors
    ///
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn resume_at(&mut self, offset: u64) -> Result<(), Error> {
        if let State::Failed(failure) = self.state {
            return Err(failure);
        }

        self.boxes = BoxReader::new();
        self.position.resume(offset);
        self.structure.resume();
        self.samples.clear();
        self.decode_times = TrackDecodeTimes::unknown();
        self.open = None;
        self.state = State::Reading;

        Ok(())
    }

    /// Declares the segment over
    ///
    /// # Errors
    ///
    /// * [`Sequence`](crate::ErrorKind::Sequence): the segment ended
    ///   inside a box.
    /// * [`Box`](crate::ErrorKind::Box): a box read into a value,
    ///   declaring no total, does not decode.
    /// * [`MissingMandatoryBox`](crate::ErrorKind::MissingMandatoryBox):
    ///   the segment carried no `moof`.
    /// * [`Sample`](crate::ErrorKind::Sample): what the samples make
    ///   of a fragment or a `sidx` declaring no total, or a sample a fragment
    ///   declared is short of the data it claimed.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   segment was already declared over.
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.reading()?;
        self.boxes
            .finish()
            .map_err(|failure| self.fail(failure.into()))?;
        self.read_framed()?;
        self.structure
            .finish()
            .map_err(|failure| self.fail(failure))?;
        self.samples
            .finish()
            .map_err(|failure| self.fail(failure.into()))?;
        self.state = State::Finished;

        Ok(())
    }

    /// Returns `Ok` while the reader still takes what arrives
    const fn reading(&self) -> Result<(), Error> {
        match self.state {
            State::Reading => Ok(()),
            State::Finished => Err(Error::already_finished()),
            State::Failed(failure) => Err(failure),
        }
    }

    /// Reads every box the framing has finished framing so far
    fn read_framed(&mut self) -> Result<(), Error> {
        while let Some(event) = self.boxes.poll_event() {
            // Why not unreachable: an event was taken, so the framing names the
            // bytes it was read from, and the fallback is a degenerate position
            // in place of a panic the lints forbid.
            let start = self
                .position
                .file_offset(self.boxes.event_extent().map_or(0, |extent| extent.start));
            match event {
                BoxEvent::Header(header) => self
                    .structure
                    .handle_box_type(header.box_type())
                    .and_then(|disposition| {
                        self.open = match disposition {
                            MediaSegmentDisposition::SegmentType => Some(Open::SegmentType(
                                WholeBoxReader::begin(header, self.payload_limit)?,
                            )),
                            MediaSegmentDisposition::MovieFragment => Some(Open::MovieFragment {
                                reader: WholeBoxReader::begin(header, self.payload_limit)?,
                                moof_start: start,
                            }),
                            MediaSegmentDisposition::SegmentIndex => Some(Open::SegmentIndex(
                                WholeBoxReader::begin(header, self.payload_limit)?,
                            )),
                            MediaSegmentDisposition::MediaData => Some(Open::MediaData),
                            MediaSegmentDisposition::Skip => None,
                        };

                        Ok(())
                    }),
                BoxEvent::Payload(payload) => match &mut self.open {
                    Some(Open::SegmentType(reader)) => reader.handle_payload(payload),
                    Some(Open::MovieFragment { reader, .. }) => reader.handle_payload(payload),
                    Some(Open::SegmentIndex(reader)) => reader.handle_payload(payload),
                    Some(Open::MediaData) => self
                        .samples
                        .handle_data(start, &payload)
                        .map_err(Error::from),
                    None => Ok(()),
                },
                BoxEvent::End => match self.open.take() {
                    Some(Open::SegmentType(reader)) => reader
                        .finish()
                        .map(|segment_type| self.segment_type = Some(segment_type)),
                    Some(Open::MovieFragment { reader, moof_start }) => {
                        reader.finish().and_then(|movie_fragment| {
                            let extents = sample_extents(
                                &movie_fragment,
                                &self.movie,
                                moof_start,
                                &mut self.decode_times,
                            )?;
                            self.samples.handle_sample_extents(extents)?;

                            Ok(())
                        })
                    }
                    Some(Open::SegmentIndex(reader)) => reader.finish().and_then(|sidx| {
                        let segment_index = subsegments(&sidx, start)?;
                        if !self.segment_indexes.contains(&segment_index) {
                            self.segment_indexes.push(segment_index);
                        }

                        Ok(())
                    }),
                    Some(Open::MediaData) | None => Ok(()),
                },
                // Why an arm at all: `BoxEvent` is `#[non_exhaustive]`, which
                // `clippy::exhaustive_enums` asks of every public enum, so §4.2
                // being settled at three steps does not close the match.
                _later_step => Ok(()),
            }
            .map_err(|failure| self.fail(failure))?;
        }

        Ok(())
    }

    /// Fails the reader for good, and hands the failure back to report
    const fn fail(&mut self, failure: Error) -> Error {
        self.state = State::Failed(failure);

        failure
    }
}

#[cfg(test)]
mod tests {
    use isobmff_boxes::{MediaDataBox, MovieFragmentBox};
    use isobmff_core::{BoxDefinition, BoxType};
    use isobmff_sample::{Sample, SampleReader};
    use isobmff_test_support::{MEDIA_DATA, framed, movie_fragment, segment_type, written};

    use super::super::tests::{movie, sample, segment_of_one_sample};
    use super::{Error, MediaSegmentDemuxFsm};
    use crate::ErrorKind;
    use crate::WantedInput;

    /// What the reader makes of `segment` handed over whole, then declared over
    fn read(segment: &[u8]) -> Result<MediaSegmentDemuxFsm, Error> {
        let mut demux_fsm = MediaSegmentDemuxFsm::new(movie());

        demux_fsm.handle_input(0, segment)?;
        demux_fsm.finish()?;

        Ok(demux_fsm)
    }

    #[test]
    fn a_segment_declaring_no_brands_is_read_all_the_same() {
        let demux_fsm = read(&written(&movie_fragment())).unwrap();

        assert_eq!(demux_fsm.segment_type(), None);
    }

    #[test]
    fn a_segment_declared_over_without_a_fragment_is_rejected() {
        assert_eq!(
            read(&written(&segment_type())).map(drop),
            Err(Error::missing_mandatory_box(MovieFragmentBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_box_read_into_a_value_declaring_a_payload_past_the_limit_is_rejected() {
        let mut demux_fsm =
            MediaSegmentDemuxFsm::with_limits(movie(), 4, SampleReader::DEFAULT_SAMPLE_SIZE_LIMIT);

        assert_eq!(
            demux_fsm
                .handle_input(0, &written(&segment_type()))
                .map_err(Error::kind),
            Err(ErrorKind::PayloadLimitExceeded)
        );
    }

    #[test]
    fn a_box_passed_over_is_not_bounded_by_the_limit() {
        let fragment = written(&movie_fragment());
        let segment = [
            fragment.clone(),
            framed(BoxType::compact(*b"free"), &[0x11; 4_096]),
        ]
        .concat();
        let mut demux_fsm = MediaSegmentDemuxFsm::with_limits(
            movie(),
            fragment.len() as u64,
            SampleReader::DEFAULT_SAMPLE_SIZE_LIMIT,
        );

        demux_fsm.handle_input(0, &segment).unwrap();

        assert_eq!(demux_fsm.finish(), Ok(()));
    }

    #[test]
    fn the_samples_completed_before_a_framing_failure_are_still_taken() {
        let mut segment = segment_of_one_sample();
        segment.extend_from_slice(b"\0\0\0\x04free");

        let mut demux_fsm = MediaSegmentDemuxFsm::new(movie());

        assert_eq!(
            demux_fsm.handle_input(0, &segment).map_err(Error::kind),
            Err(ErrorKind::Sequence(isobmff_sequence::ErrorKind::Box(
                isobmff_core::ErrorKind::SizeBelowHeader
            )))
        );
        assert_eq!(
            demux_fsm.poll_sample().map(Sample::into_data),
            Some(b"SAMP".to_vec())
        );
        assert_eq!(demux_fsm.wanted_input(), None);
    }

    #[test]
    fn media_data_the_input_is_still_to_bring_is_not_wanted_and_completes_the_sample_as_it_arrives()
    {
        let mut segment = segment_of_one_sample();
        let media_data = segment.split_off(segment.len().saturating_sub(4));

        let mut demux_fsm = MediaSegmentDemuxFsm::new(movie());
        demux_fsm.handle_input(0, &segment).unwrap();
        let wanted = demux_fsm.wanted_input();
        demux_fsm
            .handle_input(segment.len() as u64, &media_data)
            .unwrap();

        assert_eq!(wanted, Some(WantedInput::new(segment.len() as u64, None)));
        assert_eq!(demux_fsm.poll_sample(), Some(sample()));
    }

    #[test]
    fn a_failed_demux_fsm_reports_the_same_failure_for_every_call_after_it() {
        let mut demux_fsm = MediaSegmentDemuxFsm::new(movie());
        let failure = Error::box_out_of_order(MediaDataBox::BOX_TYPE);
        let segment = written(&MediaDataBox::new(MEDIA_DATA.to_vec()));

        assert_eq!(demux_fsm.handle_input(0, &segment), Err(failure));
        assert_eq!(
            demux_fsm.handle_input(0, &written(&movie_fragment())),
            Err(failure)
        );
        assert_eq!(demux_fsm.finish(), Err(failure));
    }

    #[test]
    fn input_handed_over_after_finishing_is_rejected() {
        let mut demux_fsm = read(&written(&movie_fragment())).unwrap();

        assert_eq!(
            demux_fsm.handle_input(0, &written(&movie_fragment())),
            Err(Error::already_finished())
        );
        assert_eq!(demux_fsm.finish(), Err(Error::already_finished()));
    }

    #[test]
    fn input_at_an_offset_neither_in_order_nor_wanted_is_refused_and_the_segment_reads_on() {
        let segment = segment_of_one_sample();
        let mut demux_fsm = MediaSegmentDemuxFsm::new(movie());

        demux_fsm
            .handle_input(0, segment.get(..8).unwrap())
            .unwrap();

        assert_eq!(
            demux_fsm.handle_input(9, segment.get(9..).unwrap()),
            Err(Error::unwanted_input(9))
        );
        assert_eq!(demux_fsm.handle_input(8, segment.get(8..).unwrap()), Ok(()));
        assert_eq!(demux_fsm.finish(), Ok(()));
        assert_eq!(demux_fsm.poll_sample(), Some(sample()));
    }

    #[test]
    fn empty_input_is_taken_as_nothing_wherever_it_is_handed_over() {
        let mut demux_fsm = MediaSegmentDemuxFsm::new(movie());

        assert_eq!(demux_fsm.handle_input(9, &[]), Ok(()));
        assert_eq!(demux_fsm.wanted_input(), Some(WantedInput::new(0, None)));
    }

    #[test]
    fn the_segment_is_wanted_from_the_offset_the_reading_resumed_at() {
        let mut demux_fsm = read(&segment_of_one_sample()).unwrap();

        demux_fsm.resume_at(100).unwrap();

        assert_eq!(demux_fsm.wanted_input(), Some(WantedInput::new(100, None)));
    }

    #[test]
    fn nothing_is_wanted_once_the_segment_is_declared_over() {
        let demux_fsm = read(&segment_of_one_sample()).unwrap();

        assert_eq!(demux_fsm.wanted_input(), None);
    }
}
