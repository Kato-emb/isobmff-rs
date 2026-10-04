//! [`FragmentedDemuxFsm`], a fragmented movie file read as it arrives

use alloc::vec::Vec;

use isobmff_boxes::{
    FileTypeBox, MovieBox, MovieFragmentBox, MovieFragmentRandomAccessBox,
    MovieFragmentRandomAccessOffsetBox, SegmentIndexBox,
};
use isobmff_core::{BoxDecode, BoxDefinition};
use isobmff_sample::segment_index::subsegments;
use isobmff_sample::{
    Sample, SampleReader, SegmentIndex, TrackDecodeTimes, movie_fragment, sample_table,
};
use isobmff_sequence::{BoxEvent, BoxReader};

use super::{FragmentedDisposition, FragmentedStructure};
use crate::{Error, InputPosition, InputRoute, WantedInput, WholeBoxReader};

/// Reads the samples a fragmented movie file carries, taking it as it arrives
///
/// A fragmented movie file is laid out as ISO/IEC 14496-12 Annex A.8 has it:
/// the brands it declares itself readable as, the movie its fragments
/// continue, which may declare samples of its own (§8.8), then one movie
/// fragment after another, with the media data the movie and its fragments
/// address lying anywhere among them. This demux FSM wires the layers that
/// read one: the framing of the file into boxes, the structure that says what
/// each top-level box is, the reading of the boxes it names into values, the
/// resolution of the sample tables of the movie, and of each fragment against
/// the movie, into the extents of their samples, and the gathering of those
/// samples out of the media data. It holds no rule of its
/// own but one: of the bytes the samples still lack, it names only those whose
/// start the file handed over in order has passed. A caller hands over bytes
/// and takes [`Sample`]s. It reaches for no source of its own: when to read
/// and from where stay with the caller.
///
/// # Contract
///
/// * The file is handed over from its first byte, or from the offset
///   [`resume_at`](Self::resume_at) names, in order and cut anywhere,
///   each cut handed over at the offset it was read at, where
///   [`wanted_input`](Self::wanted_input) names,
///   and the samples it completed are taken from
///   [`poll_sample`](Self::poll_sample). The caller drains before handing
///   over more: samples are held until they are taken. Where the file lies in
///   its resource is the caller's: every offset the demux FSM reports is a file
///   offset, counting from the first byte of the file as the boxes count
///   theirs (§8.7.5, §8.8.7).
/// * The boxes the structure reads into values are there to read once they
///   have arrived: [`file_type`](Self::file_type), [`movie`](Self::movie),
///   the indexes of every `sidx` as [`segment_indexes`](Self::segment_indexes)
///   and the `mfra` as
///   [`movie_fragment_random_access`](Self::movie_fragment_random_access).
///   The media data is offered to the samples, and every other box is passed
///   over.
/// * [`resume_at`](Self::resume_at) restarts the reading at a file offset an
///   index points at, a `moof`, a `sidx` or an `mfra`, from where the file is
///   then handed over. The movie and the indexes read so far stand; the
///   extents held, those of the movie among them, and the samples not yet
///   taken are dropped, and are not resolved again; where each track stands
///   on its timeline is no longer known until a `tfdt` states it; a fragment
///   stating none for such a track is
///   [`Sample`](crate::ErrorKind::Sample).
///   [`resume_at_movie_fragment_random_access`](Self::resume_at_movie_fragment_random_access)
///   restarts it at the `mfra` the file closes with, once the last bytes of
///   the file, wanted first, name it, or declares the file over where they
///   name none.
/// * The order the boxes come in, and what a file that breaks it is reported
///   as, are the structure's: an `ftyp` after another box, a
///   `moof` before the `moov` are
///   [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder), a second
///   `moov` is [`DuplicateBox`](crate::ErrorKind::DuplicateBox), and
///   a file declared over without a `moov`, other than while its closing
///   `mfro` is gathered, is
///   [`MissingMandatoryBox`](crate::ErrorKind::MissingMandatoryBox).
///   A file carrying no `ftyp` reads all the same, as §4.3 allows.
/// * A box read into a value is gathered whole before it is read, so what it
///   declares is bounded — see [`with_limits`](Self::with_limits).
/// * The samples the sample tables of the movie declare are resolved once the
///   `moov` has been read, and those of a fragment once the `moof` has; where
///   their chunks and runs lie is not checked. Either comes out as its bytes
///   arrive whole, as [`SampleReader`]'s contract has it: the extents of the
///   movie, and those of each fragment, are held in the order of their bytes,
///   so a file handed over in order yields the samples of each in the order
///   they lie in it, whatever order the boxes declare them in and wherever
///   the input is cut. [`wanted_input`](Self::wanted_input) names what the
///   extent at the front of those held still lacks only once the input has
///   passed its start, which a file whose movie and fragments precede their
///   media data never has.
/// * Where a fragment states no decode time for a track, the track goes on
///   from where the samples before it left it, those the sample table of the
///   movie declares among them (§8.8.12).
/// * An `Err` leaves the demux FSM failed for good,
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished) and
///   [`UnwantedInput`](crate::ErrorKind::UnwantedInput) aside:
///   every later call reports that same failure again. The samples completed
///   before it are still there to take.
/// * [`finish`](Self::finish) declares the file over, and reports what any
///   layer makes of the end of it: a box left open, the `moov` never come, a
///   sample short of the data it claimed; while the closing `mfro` is
///   gathered, it reports nothing. Samples are still taken after it, but
///   anything handed over then, or a second [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished), until
///   [`resume_at`](Self::resume_at) restarts the reading, or
///   [`resume_at_movie_fragment_random_access`](Self::resume_at_movie_fragment_random_access)
///   does where it locates an `mfra`.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{SampleFlags, TrackExtendsBox};
/// use isobmff_sample::Sample;
/// use isobmff_structure::{FragmentedDemuxFsm, FragmentedMuxFsm};
/// # use isobmff_test_support::{file_type, fragmented_movie};
/// // A file of one fragment carrying two samples of track 1
/// let mut mux_fsm = FragmentedMuxFsm::new();
/// mux_fsm.handle_file_type(file_type())?;
/// mux_fsm.handle_movie(fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO)))?;
/// mux_fsm.begin_fragment(1)?;
/// mux_fsm.handle_sample(Sample::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec()))?;
/// mux_fsm.handle_sample(Sample::new(1, 1_024, 1_024, 0, SampleFlags::ZERO, 1, b"DATA".to_vec()))?;
/// mux_fsm.finish_fragment()?;
/// mux_fsm.finish()?;
///
/// // The file the mux FSM laid down is drained as it hands the bytes over
/// let mut file = Vec::new();
/// while let Some(written) = mux_fsm.poll_output() {
///     file.extend_from_slice(&written);
/// }
///
/// // The file is handed over as it arrives, in whatever lengths it comes, each cut at its offset
/// let mut demux_fsm = FragmentedDemuxFsm::new();
/// for (offset, arriving) in (0..).step_by(7).zip(file.chunks(7)) {
///     demux_fsm.handle_input(offset, arriving)?;
/// }
/// demux_fsm.finish()?;
///
/// // The brands and the movie the file declared are there to read
/// assert_eq!(demux_fsm.file_type().map(|ftyp| ftyp.major_brand()), Some(file_type().major_brand()));
/// assert_eq!(demux_fsm.movie().map(|moov| moov.trak().len()), Some(1));
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
pub struct FragmentedDemuxFsm {
    boxes: BoxReader,
    position: InputPosition,
    structure: FragmentedStructure,
    samples: SampleReader,
    decode_times: TrackDecodeTimes,
    open: Option<Open>,
    file_type: Option<FileTypeBox>,
    movie: Option<MovieBox>,
    segment_indexes: Vec<SegmentIndex>,
    movie_fragment_random_access: Option<MovieFragmentRandomAccessBox>,
    payload_limit: u64,
    state: State,
}

/// Where the demux FSM stands between calls
#[derive(Clone, Copy, Debug)]
enum State {
    /// Taking the file as it arrives
    Reading,
    /// Gathering the bytes a file `file_len` long would close with an `mfro` in, and taking nothing else
    LocatingMovieFragmentRandomAccess {
        file_len: u64,
        mfro: [u8; MovieFragmentRandomAccessOffsetBox::ENCODED_LEN],
        filled: usize,
    },
    /// Told the file is over, and taking no more input
    Finished,
    /// Failed, and reporting that same failure for every call after it
    Failed(Error),
}

/// The top-level box that started, held as its disposition has it until it ends
#[derive(Debug)]
enum Open {
    /// Brands being read whole
    FileType(WholeBoxReader<FileTypeBox>),
    /// Movie being read whole
    Movie(WholeBoxReader<MovieBox>),
    /// Fragment being read whole, and where in the file it began
    MovieFragment {
        reader: WholeBoxReader<MovieFragmentBox>,
        moof_start: u64,
    },
    /// Segment index being read whole
    SegmentIndex(WholeBoxReader<SegmentIndexBox>),
    /// Random access tables being read whole
    MovieFragmentRandomAccess(WholeBoxReader<MovieFragmentRandomAccessBox>),
    /// Media data, offered to the samples as it arrives
    MediaData,
}

impl FragmentedDemuxFsm {
    /// Payload a box read into a value may declare, where the caller names no limit
    ///
    /// Sixteen mebibytes. A caller reading files whose `moov` reaches past that
    /// — a presentation of many tracks states a sample entry for each — names a
    /// limit of its own with [`with_limits`](Self::with_limits).
    pub const DEFAULT_PAYLOAD_LIMIT: u64 = 16 * 1024 * 1024;

    /// Creates a demux FSM waiting at the start of a fragmented movie file
    ///
    /// What a box read into a value may declare is bounded by
    /// [`DEFAULT_PAYLOAD_LIMIT`](Self::DEFAULT_PAYLOAD_LIMIT), and what one
    /// sample may declare by
    /// [`SampleReader::DEFAULT_SAMPLE_SIZE_LIMIT`](SampleReader::DEFAULT_SAMPLE_SIZE_LIMIT).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_limits(
            Self::DEFAULT_PAYLOAD_LIMIT,
            SampleReader::DEFAULT_SAMPLE_SIZE_LIMIT,
        )
    }

    /// Creates a demux FSM holding the file to `payload_limit` and `sample_size_limit`
    ///
    /// Both bound memory the demux FSM is about to take, and both bound one box
    /// or one sample rather than the file. A box read into a value that
    /// declares more than `payload_limit` bytes of payload is
    /// [`PayloadLimitExceeded`](crate::ErrorKind::PayloadLimitExceeded)
    /// before a byte of it is gathered; a sample declaring more than
    /// `sample_size_limit` bytes is what
    /// [`SampleReader::with_sample_size_limit`](SampleReader::with_sample_size_limit)
    /// makes of it.
    #[must_use]
    pub const fn with_limits(payload_limit: u64, sample_size_limit: u64) -> Self {
        Self {
            boxes: BoxReader::new(),
            position: InputPosition::new(),
            structure: FragmentedStructure::new(),
            samples: SampleReader::with_sample_size_limit(sample_size_limit),
            decode_times: TrackDecodeTimes::unknown(),
            open: None,
            file_type: None,
            movie: None,
            segment_indexes: Vec::new(),
            movie_fragment_random_access: None,
            payload_limit,
            state: State::Reading,
        }
    }

    /// Takes bytes of the file read at `offset`, and reads the samples they complete
    ///
    /// `offset` is a file offset, counted from the first byte of the file as a
    /// base data offset is (ISO/IEC 14496-12 §8.8.7). Bytes at the offset the
    /// input taken in order stands at — the first byte of the file or the
    /// offset the last [`resume_at`](Self::resume_at) named, then where the
    /// bytes taken in order before them end — are taken whole as its
    /// continuation. Bytes at the offset of the bytes
    /// [`wanted_input`](Self::wanted_input) names as lacking are offered to the
    /// samples alone, as media data is, and the input taken in order goes on
    /// from where it stood; bytes at the offset the input taken in order stands
    /// at are taken all the same while such bytes are named. While the bytes
    /// the file would close with an `mfro` in are gathered, only they are
    /// taken, and none past them. Empty input is taken as nothing. What the
    /// input completed is then taken from [`poll_sample`](Self::poll_sample).
    ///
    /// # Errors
    ///
    /// * [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder),
    ///   [`DuplicateBox`](crate::ErrorKind::DuplicateBox): what the
    ///   structure makes of a top-level box arriving where it does.
    /// * [`PayloadLimitExceeded`](crate::ErrorKind::PayloadLimitExceeded):
    ///   a box read into a value reaches past the limit the demux FSM gathers.
    /// * [`Sequence`](crate::ErrorKind::Sequence): what the framing
    ///   of the file makes of the input.
    /// * [`Box`](crate::ErrorKind::Box): a box read into a value
    ///   does not decode.
    /// * [`Sample`](crate::ErrorKind::Sample): what the samples make
    ///   of the movie, a fragment, a `sidx`, or the media data beside them.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was declared over, by [`finish`](Self::finish) or by
    ///   [`resume_at_movie_fragment_random_access`](Self::resume_at_movie_fragment_random_access).
    /// * [`UnwantedInput`](crate::ErrorKind::UnwantedInput): `offset` is
    ///   not where [`wanted_input`](Self::wanted_input) names, nor, while the
    ///   input is taken in order, where it stands. The demux FSM is not failed
    ///   by it.
    /// * The failure of a previous call, which the demux FSM keeps and reports
    ///   again for every call after it.
    pub fn handle_input(&mut self, offset: u64, input: &[u8]) -> Result<(), Error> {
        match &mut self.state {
            State::Reading | State::LocatingMovieFragmentRandomAccess { .. }
                if input.is_empty() =>
            {
                return Ok(());
            }
            State::Reading => {}
            State::LocatingMovieFragmentRandomAccess {
                file_len,
                mfro,
                filled,
            } => {
                if offset != closing_offset(*file_len, *filled) {
                    return Err(Error::unwanted_input(offset));
                }
                let rest = mfro.get_mut(*filled..).unwrap_or_default();
                let taken = rest.len().min(input.len());
                rest.iter_mut()
                    .zip(input)
                    .for_each(|(slot, byte)| *slot = *byte);
                *filled = filled.saturating_add(taken);
                if *filled < MovieFragmentRandomAccessOffsetBox::ENCODED_LEN {
                    return Ok(());
                }

                match MovieFragmentRandomAccessOffsetBox::decode(mfro.as_slice())
                    .ok()
                    .and_then(|(mfro, _)| mfro.movie_fragment_random_access_start(*file_len))
                {
                    Some(movie_fragment_random_access_start) => {
                        self.restart(movie_fragment_random_access_start);
                    }
                    None => self.state = State::Finished,
                }

                return Ok(());
            }
            State::Finished => return Err(Error::already_finished()),
            State::Failed(failure) => return Err(*failure),
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

    /// Takes the next sample the file handed over so far completed
    ///
    /// Reports `None` once they are used up: more of the file is needed. Failure
    /// is reported by the calls that take it, so this one never fails — a failed
    /// demux FSM hands over the samples it had already completed, then `None`
    /// from there on.
    pub fn poll_sample(&mut self) -> Option<Sample> {
        self.samples.poll_sample()
    }

    /// Returns the one read wanted next, or `None` once the file is declared over or the demux FSM has failed
    ///
    /// Bytes the extent at the front of those held still lacks are wanted
    /// first, with their length, once the input taken in order has passed their
    /// start. Where the movie and each fragment precede the media data they
    /// address, a file handed over in order has none wanted; a movie whose
    /// chunks lie before it (§8.1.1 sets no order for an `mdat`), or a
    /// fragment addressing media data lying before it (§8.8.7 has a base data
    /// offset name any byte of the file), wants bytes already passed by.
    /// Otherwise the continuation of the input taken in order is wanted, at
    /// the offset it stands at and with no length. While the bytes the file would close with an `mfro` in are gathered, after
    /// [`resume_at_movie_fragment_random_access`](Self::resume_at_movie_fragment_random_access),
    /// those still to gather are wanted alone, with their length. Bytes read
    /// for any of these are handed to [`handle_input`](Self::handle_input) at
    /// the offset they were read at.
    #[must_use]
    pub fn wanted_input(&self) -> Option<WantedInput> {
        match self.state {
            State::Reading => Some(self.position.wanted_input(self.samples.wanted_extent())),
            State::LocatingMovieFragmentRandomAccess {
                file_len, filled, ..
            } => {
                let start = closing_offset(file_len, filled);

                Some(WantedInput::new(
                    start,
                    Some(file_len.saturating_sub(start)),
                ))
            }
            State::Finished | State::Failed(_) => None,
        }
    }

    /// Returns the brands the file declares itself readable as, once they have arrived
    #[must_use]
    pub const fn file_type(&self) -> Option<&FileTypeBox> {
        self.file_type.as_ref()
    }

    /// Returns the movie the fragments of the file continue, once it has arrived
    #[must_use]
    pub const fn movie(&self) -> Option<&MovieBox> {
        self.movie.as_ref()
    }

    /// Returns the subsegments of every `sidx` read so far, in the order they were read
    ///
    /// A `sidx` read again, as the reading resumes at or before it, is held
    /// once. Each is placed in the file from the first byte after its `sidx`, as
    /// [`subsegments`] places them.
    #[must_use]
    pub fn segment_indexes(&self) -> &[SegmentIndex] {
        &self.segment_indexes
    }

    /// Returns the random access tables of the file, once its `mfra` has been read
    ///
    /// An `mfra` read again, as the file is read on to its end after a
    /// resume, takes the place of the one read before it.
    #[must_use]
    pub const fn movie_fragment_random_access(&self) -> Option<&MovieFragmentRandomAccessBox> {
        self.movie_fragment_random_access.as_ref()
    }

    /// Restarts the reading at `offset`, the file offset the input handed over next starts at
    ///
    /// The next box is to be one an index points at: a `moof`, a `sidx` or an
    /// `mfra`. The demux FSM resumes from reading and from the file declared
    /// over alike, and takes the file again from there.
    ///
    /// # Errors
    ///
    /// * The failure of a previous call, which the demux FSM keeps and reports
    ///   again for every call after it.
    pub fn resume_at(&mut self, offset: u64) -> Result<(), Error> {
        if let State::Failed(failure) = self.state {
            return Err(failure);
        }
        self.restart(offset);

        Ok(())
    }

    /// Restarts the reading at the `mfra` a file `file_len` bytes long closes with, once its last bytes are read
    ///
    /// The last bytes of the file are wanted next, as many as an `mfro`
    /// occupies (ISO/IEC 14496-12 §8.8.11), and taken through
    /// [`handle_input`](Self::handle_input) however they are cut. Where they are
    /// an `mfro` stepping back within the file, the reading restarts at the
    /// offset it names, as [`resume_at`](Self::resume_at) restarts it, and the
    /// file is wanted from there, its `mfra` read as
    /// [`movie_fragment_random_access`](Self::movie_fragment_random_access).
    /// Where the file is shorter than an `mfro`, closes with none, or closes
    /// with one stepping back past its start, the file is declared over: no
    /// read is wanted, the `mfra` read before stays, and
    /// [`resume_at`](Self::resume_at) takes the reading up again. Until then,
    /// input at any offset but the one [`wanted_input`](Self::wanted_input)
    /// names is [`UnwantedInput`](crate::ErrorKind::UnwantedInput), and
    /// [`finish`](Self::finish) declares the file over with no failure. The
    /// demux FSM is told this from reading and from the file declared over
    /// alike; what [`resume_at`](Self::resume_at) drops is dropped only once
    /// the `mfra` is located.
    ///
    /// # Errors
    ///
    /// * The failure of a previous call, which the demux FSM keeps and reports
    ///   again for every call after it.
    pub fn resume_at_movie_fragment_random_access(&mut self, file_len: u64) -> Result<(), Error> {
        if let State::Failed(failure) = self.state {
            return Err(failure);
        }

        self.state = if file_len < MovieFragmentRandomAccessOffsetBox::ENCODED_LEN as u64 {
            State::Finished
        } else {
            State::LocatingMovieFragmentRandomAccess {
                file_len,
                mfro: [0; MovieFragmentRandomAccessOffsetBox::ENCODED_LEN],
                filled: 0,
            }
        };

        Ok(())
    }

    /// Declares the file over
    ///
    /// While the bytes the file would close with an `mfro` in are gathered, after
    /// [`resume_at_movie_fragment_random_access`](Self::resume_at_movie_fragment_random_access),
    /// the file is declared over with no failure.
    ///
    /// # Errors
    ///
    /// * [`Sequence`](crate::ErrorKind::Sequence): the file ended
    ///   inside a box.
    /// * [`Box`](crate::ErrorKind::Box): a box read into a value,
    ///   declaring no total, does not decode.
    /// * [`MissingMandatoryBox`](crate::ErrorKind::MissingMandatoryBox):
    ///   the file carried no `moov`.
    /// * [`Sample`](crate::ErrorKind::Sample): what the samples make
    ///   of the movie, a fragment or a `sidx` declaring no total, or a sample
    ///   the movie or a fragment declared is short of the data it claimed.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was already declared over.
    /// * The failure of a previous call, which the demux FSM keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<(), Error> {
        match self.state {
            State::Reading => {}
            State::LocatingMovieFragmentRandomAccess { .. } => {
                self.state = State::Finished;

                return Ok(());
            }
            State::Finished => return Err(Error::already_finished()),
            State::Failed(failure) => return Err(failure),
        }
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

    /// Restarts the reading at `offset`, whatever the demux FSM stood at
    fn restart(&mut self, offset: u64) {
        self.boxes = BoxReader::new();
        self.position.resume(offset);
        self.structure.resume();
        self.samples.clear();
        self.decode_times = TrackDecodeTimes::unknown();
        self.open = None;
        self.state = State::Reading;
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
                            FragmentedDisposition::FileType => Some(Open::FileType(
                                WholeBoxReader::begin(header, self.payload_limit)?,
                            )),
                            FragmentedDisposition::Movie => Some(Open::Movie(
                                WholeBoxReader::begin(header, self.payload_limit)?,
                            )),
                            FragmentedDisposition::MovieFragment => Some(Open::MovieFragment {
                                reader: WholeBoxReader::begin(header, self.payload_limit)?,
                                moof_start: start,
                            }),
                            FragmentedDisposition::SegmentIndex => Some(Open::SegmentIndex(
                                WholeBoxReader::begin(header, self.payload_limit)?,
                            )),
                            FragmentedDisposition::MovieFragmentRandomAccess => {
                                Some(Open::MovieFragmentRandomAccess(WholeBoxReader::begin(
                                    header,
                                    self.payload_limit,
                                )?))
                            }
                            FragmentedDisposition::MediaData => Some(Open::MediaData),
                            FragmentedDisposition::Skip => None,
                        };

                        Ok(())
                    }),
                BoxEvent::Payload(payload) => match &mut self.open {
                    Some(Open::FileType(reader)) => reader.handle_payload(payload),
                    Some(Open::Movie(reader)) => reader.handle_payload(payload),
                    Some(Open::MovieFragment { reader, .. }) => reader.handle_payload(payload),
                    Some(Open::SegmentIndex(reader)) => reader.handle_payload(payload),
                    Some(Open::MovieFragmentRandomAccess(reader)) => reader.handle_payload(payload),
                    Some(Open::MediaData) => self
                        .samples
                        .handle_data(start, &payload)
                        .map_err(Error::from),
                    None => Ok(()),
                },
                BoxEvent::End => match self.open.take() {
                    Some(Open::FileType(reader)) => reader
                        .finish()
                        .map(|file_type| self.file_type = Some(file_type)),
                    Some(Open::Movie(reader)) => reader.finish().and_then(|movie| {
                        self.samples
                            .handle_sample_extents(sample_table::sample_extents(&movie))?;
                        self.decode_times = TrackDecodeTimes::new(&movie)?;
                        self.movie = Some(movie);

                        Ok(())
                    }),
                    Some(Open::MovieFragment { reader, moof_start }) => {
                        reader.finish().and_then(|movie_fragment| {
                            // Why not unreachable: the structure placed the `moof`
                            // after the `moov`, so the movie is there, and the
                            // fallback repeats what the structure answers a `moof`
                            // before it with, in place of a panic the lints forbid.
                            let Some(movie) = self.movie.as_ref() else {
                                return Err(Error::box_out_of_order(MovieFragmentBox::BOX_TYPE));
                            };

                            let extents = movie_fragment::sample_extents(
                                &movie_fragment,
                                movie,
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
                    Some(Open::MovieFragmentRandomAccess(reader)) => reader
                        .finish()
                        .map(|mfra| self.movie_fragment_random_access = Some(mfra)),
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

    /// Fails the demux FSM for good, and hands the failure back to report
    const fn fail(&mut self, failure: Error) -> Error {
        self.state = State::Failed(failure);

        failure
    }
}

/// Returns the file offset the bytes still to gather of the `mfro` closing a file `file_len` long begin at, `filled` of them gathered
const fn closing_offset(file_len: u64, filled: usize) -> u64 {
    let left = MovieFragmentRandomAccessOffsetBox::ENCODED_LEN.saturating_sub(filled);

    file_len.saturating_sub(left as u64)
}

impl Default for FragmentedDemuxFsm {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
