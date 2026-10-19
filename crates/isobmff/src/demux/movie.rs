//! [`MovieDemuxer`], a movie file read as it arrives, fragmented or not, ISO/IEC 14496-12 §8.2.1, §8.7 and Annex A.8

use alloc::vec::Vec;

use isobmff_boxes::{
    FileTypeBox, MovieBox, MovieFragmentBox, MovieFragmentRandomAccessBox,
    MovieFragmentRandomAccessOffsetBox, SegmentIndexBox,
};
use isobmff_core::{BoxDecode, BoxDefinition};

use crate::demux::{
    DemuxError, DemuxInput, DemuxLimits, SampleGathering, WantedInput, WholeBoxReader,
};
use crate::sample::{SampleReader, SegmentIndex, TrackDecodeTimes, movie_fragment, sample_table};
use crate::sequence::BoxEvent;
use crate::structure::{FragmentedDisposition, FragmentedStructure};

/// Reads a movie file as it arrives, fragmented or not, into the samples it carries or their extents
///
/// A movie file declares its samples in the sample tables of its one movie
/// (ISO/IEC 14496-12 §8.2.1). A fragmented one is laid out as Annex A.8 has
/// it: the brands it declares itself readable as, the movie its fragments
/// continue, which may declare samples of its own (§8.8), then one movie
/// fragment after another. Either way the media data the movie and its
/// fragments address lies anywhere among them, before the movie as well as
/// after it. This demuxer wires the layers that
/// read one: the framing of the file into boxes, the structure that says what
/// each top-level box is, the reading of the boxes it names into values, the
/// resolution of the sample tables of the movie, and of each fragment against
/// the movie, into the extents of their samples, and what becomes of those
/// extents, which is `G`'s, a [`SampleGathering`]: with [`SampleReader`],
/// the default, the samples are gathered out of the media data, and a caller
/// hands over bytes and takes [`Sample`](crate::sample::Sample)s; with
/// [`SampleExtents`](crate::SampleExtents), the extents come out
/// as they are, the media data passed over, and a caller takes
/// [`SampleExtent`](crate::sample::SampleExtent)s. It holds no rule of its
/// own but one: of the bytes the samples still lack, it names only those
/// whose start the file handed over in order has passed. It reaches for no
/// source of its own: when to read and from where stay with the caller.
///
/// # Contract
///
/// * The file is handed over from its first byte, or from the offset
///   [`resume_at`](Self::resume_at) names, in order and cut anywhere,
///   each cut handed over at the offset it was read at, where
///   [`wanted_input`](Self::wanted_input) names,
///   and what it completed is taken from
///   [`poll_output`](Self::poll_output). The caller drains before handing
///   over more: output is held until it is taken. Where the file lies in
///   its resource is the caller's: every offset the demuxer reports is a file
///   offset, counting from the first byte of the file as the boxes count
///   theirs (§8.7.5, §8.8.7).
/// * The boxes the structure reads into values are there to read once they
///   have arrived: [`file_type`](Self::file_type), [`movie`](Self::movie),
///   the indexes of every `sidx` as [`segment_indexes`](Self::segment_indexes)
///   and the `mfra` as
///   [`movie_fragment_random_access`](Self::movie_fragment_random_access).
///   The media data is offered to the gathering, and every other box is
///   passed over.
/// * [`resume_at`](Self::resume_at) restarts the reading at a file offset an
///   index points at, a `moof`, a `sidx` or an `mfra`, from where the file is
///   then handed over. The movie and the indexes read so far stand; the
///   extents held, those of the movie among them, and the samples not yet
///   taken are dropped, and are not resolved again; where each track stands
///   on its timeline is no longer known until a `tfdt` states it; a fragment
///   stating none for such a track is
///   [`Sample`](crate::DemuxError::Sample).
///   [`resume_at_movie_fragment_random_access`](Self::resume_at_movie_fragment_random_access)
///   restarts it at the `mfra` the file closes with, once the last bytes of
///   the file, wanted first, name it, or declares the file over where they
///   name none.
/// * The order the boxes come in, and what a file that breaks it is reported
///   as, are the structure's: an `ftyp` after another box, a
///   `moof` before the `moov` are
///   [`BoxOutOfOrder`](crate::DemuxError::BoxOutOfOrder), a second
///   `moov` is [`DuplicateBox`](crate::DemuxError::DuplicateBox), and
///   a file declared over without a `moov`, other than while its closing
///   `mfro` is gathered, is
///   [`MissingMandatoryBox`](crate::DemuxError::MissingMandatoryBox).
///   A file carrying no `ftyp` reads all the same, as §4.3 allows.
/// * What the demuxer holds of what the file declares is bounded — a box
///   read into a value, the samples a `moov` or a `moof` declares — by the
///   [`DemuxLimits`] it was created with; what the gathering holds is bounded
///   by what it was created with, [`DemuxLimits::sample_reader`] where
///   [`with_limits`](Self::with_limits) created it.
/// * The samples the sample tables of the movie declare are resolved once the
///   `moov` has been read, and those of a fragment once the `moof` has; where
///   their chunks and runs lie is not checked. Gathering with
///   [`SampleReader`], either comes out as its bytes arrive whole, as its
///   contract has it: the extents of the movie, and those of each fragment,
///   are held in the order of their bytes, so a file handed over in order
///   yields the samples of each in the order they lie in it, whatever order
///   the boxes declare them in and wherever the input is cut.
///   [`wanted_input`](Self::wanted_input) names what the extent at the front
///   of those held still lacks only once the input has passed its start,
///   which a file whose movie and fragments precede their media data never
///   has. Gathering with [`SampleExtents`](crate::SampleExtents),
///   the extents come out as each `moov` or `moof` is read, and nothing is
///   wanted but the continuation.
/// * A fragment continues a movie that declares it may be fragmented: a
///   `moof` after a `moov` carrying no `mvex` is
///   [`Sample`](crate::DemuxError::Sample), the movie extends box missing
///   (§8.8.1).
/// * Where a fragment states no decode time for a track, the track goes on
///   from where the samples before it left it, those the sample table of the
///   movie declares among them (§8.8.12).
/// * An `Err` leaves the demuxer failed for good,
///   [`AlreadyFinished`](crate::DemuxError::AlreadyFinished) and
///   [`UnwantedInput`](crate::DemuxError::UnwantedInput) aside:
///   every later call reports that same failure again. The output completed
///   before it is still there to take.
/// * [`finish`](Self::finish) declares the file over, and reports what any
///   layer makes of the end of it: a box left open, the `moov` never come, a
///   sample short of the data it claimed; while the closing `mfro` is
///   gathered, it reports nothing. Output is still taken after it, but
///   anything handed over then, or a second [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::DemuxError::AlreadyFinished), until
///   [`resume_at`](Self::resume_at) restarts the reading, or
///   [`resume_at_movie_fragment_random_access`](Self::resume_at_movie_fragment_random_access)
///   does where it locates an `mfra`.
///
/// # Examples
///
/// ```
/// use isobmff::boxes::{SampleFlags, TrackExtendsBox};
/// use isobmff::sample::{Sample, SampleProperties};
/// use isobmff::{FragmentedMuxer, MovieDemuxer};
/// # use isobmff_test_support::{file_type, fragmented_movie};
/// // A file of one fragment carrying two samples of track 1
/// let mut muxer = FragmentedMuxer::new();
/// muxer.handle_file_type(file_type())?;
/// muxer.handle_movie(fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO)))?;
/// muxer.begin_fragment(1)?;
/// muxer.handle_sample(Sample::new(
///     SampleProperties {
///         track_id: 1,
///         decode_time: 0,
///         sample_duration: 1_024,
///         sample_composition_time_offset: 0,
///         sample_flags: SampleFlags::ZERO,
///         sample_description_index: 1,
///     },
///     b"SAMP".to_vec(),
/// ))?;
/// muxer.handle_sample(Sample::new(
///     SampleProperties {
///         track_id: 1,
///         decode_time: 1_024,
///         sample_duration: 1_024,
///         sample_composition_time_offset: 0,
///         sample_flags: SampleFlags::ZERO,
///         sample_description_index: 1,
///     },
///     b"DATA".to_vec(),
/// ))?;
/// muxer.finish_fragment()?;
/// muxer.finish()?;
///
/// // The file the muxer laid down is drained as it hands the bytes over
/// let mut file = Vec::new();
/// while let Some(written) = muxer.poll_output() {
///     file.extend_from_slice(&written);
/// }
///
/// // The file is handed over as it arrives, in whatever lengths it comes, each cut at its offset
/// let mut demuxer = MovieDemuxer::new();
/// for (offset, arriving) in (0..).step_by(7).zip(file.chunks(7)) {
///     demuxer.handle_input(offset, arriving)?;
/// }
/// demuxer.finish()?;
///
/// // The brands and the movie the file declared are there to read
/// assert_eq!(demuxer.file_type().map(|ftyp| ftyp.major_brand()), Some(file_type().major_brand()));
/// assert_eq!(demuxer.movie().map(|moov| moov.trak().len()), Some(1));
///
/// // The samples come back as they were laid out
/// let first = demuxer.poll_output().unwrap();
/// assert_eq!((first.data(), first.properties().decode_time), (b"SAMP".as_slice(), 0));
/// let second = demuxer.poll_output().unwrap();
/// assert_eq!((second.data(), second.properties().decode_time), (b"DATA".as_slice(), 1_024));
/// assert_eq!(demuxer.poll_output(), None);
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct MovieDemuxer<G = SampleReader> {
    input: DemuxInput<G>,
    structure: FragmentedStructure,
    decode_times: TrackDecodeTimes,
    open: Option<Open>,
    file_type: Option<FileTypeBox>,
    movie: Option<MovieBox>,
    segment_indexes: Vec<SegmentIndex>,
    movie_fragment_random_access: Option<MovieFragmentRandomAccessBox>,
    limits: DemuxLimits,
    state: State,
}

/// Where the demuxer stands between calls
#[derive(Clone, Copy, Debug)]
enum State {
    /// Taking the file as the input half does, gathering no `mfro`
    Reading,
    /// Gathering the bytes a file `file_len` long would close with an `mfro` in, and taking nothing else
    LocatingMovieFragmentRandomAccess {
        file_len: u64,
        mfro: [u8; MovieFragmentRandomAccessOffsetBox::ENCODED_LEN],
        filled: usize,
    },
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

impl MovieDemuxer {
    /// Creates a demuxer waiting at the start of a movie file, gathering its samples, bounded by the limits [`DemuxLimits::new`] states
    #[must_use]
    pub const fn new() -> Self {
        Self::with_limits(DemuxLimits::new())
    }

    /// Creates a demuxer waiting at the start of a movie file, gathering its samples, bounded by `limits`
    ///
    /// The samples are gathered by a [`SampleReader`] bounded by
    /// [`DemuxLimits::sample_reader`].
    #[must_use]
    pub const fn with_limits(limits: DemuxLimits) -> Self {
        Self::with_gathering(SampleReader::with_limits(limits.sample_reader()), limits)
    }
}

impl<G: SampleGathering> MovieDemuxer<G> {
    /// Creates a demuxer waiting at the start of a movie file, `gathering` taking the extents it resolves, bounded by `limits`
    ///
    /// Of `limits`, the payload a box read into a value may reach and the
    /// samples one `moov` or `moof` may declare apply; `gathering` is taken as
    /// it stands, with whatever bounds it was created with, so
    /// [`DemuxLimits::sample_reader`] does not.
    ///
    /// # Examples
    ///
    /// ```
    /// use isobmff::boxes::{SampleFlags, TrackExtendsBox};
    /// use isobmff::sample::{Sample, SampleProperties};
    /// use isobmff::{DemuxLimits, FragmentedMuxer, MovieDemuxer, SampleExtents};
    /// # use isobmff_test_support::{file_type, fragmented_movie};
    /// // A file of one fragment carrying one sample of track 1
    /// let mut muxer = FragmentedMuxer::new();
    /// muxer.handle_file_type(file_type())?;
    /// muxer.handle_movie(fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO)))?;
    /// muxer.begin_fragment(1)?;
    /// muxer.handle_sample(Sample::new(
    ///     SampleProperties {
    ///         track_id: 1,
    ///         decode_time: 0,
    ///         sample_duration: 1_024,
    ///         sample_composition_time_offset: 0,
    ///         sample_flags: SampleFlags::ZERO,
    ///         sample_description_index: 1,
    ///     },
    ///     b"SAMP".to_vec(),
    /// ))?;
    /// muxer.finish_fragment()?;
    /// muxer.finish()?;
    ///
    /// // The bytes the muxer wrote are the file
    /// let mut file = Vec::new();
    /// while let Some(written) = muxer.poll_output() {
    ///     file.extend_from_slice(&written);
    /// }
    ///
    /// // The file is handed over in order; the extents come out, the media data passed over
    /// let mut demuxer = MovieDemuxer::with_gathering(SampleExtents::new(), DemuxLimits::new());
    /// for (offset, arriving) in (0..).step_by(7).zip(file.chunks(7)) {
    ///     demuxer.handle_input(offset, arriving)?;
    /// }
    /// demuxer.finish()?;
    ///
    /// // The extent names the bytes of the sample in the file
    /// let extent = demuxer.poll_output().unwrap();
    /// let range = usize::try_from(extent.extent().start).unwrap()..usize::try_from(extent.extent().end).unwrap();
    /// assert_eq!(&file[range], b"SAMP");
    /// assert_eq!(demuxer.poll_output(), None);
    /// # Ok::<(), Box<dyn core::error::Error>>(())
    /// ```
    #[must_use]
    pub const fn with_gathering(gathering: G, limits: DemuxLimits) -> Self {
        Self {
            input: DemuxInput::new(gathering),
            structure: FragmentedStructure::new(),
            decode_times: TrackDecodeTimes::unknown(),
            open: None,
            file_type: None,
            movie: None,
            segment_indexes: Vec::new(),
            movie_fragment_random_access: None,
            limits,
            state: State::Reading,
        }
    }

    /// Takes bytes of the file read at `offset`, and completes the output they carry
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
    /// input completed is then taken from [`poll_output`](Self::poll_output).
    ///
    /// # Errors
    ///
    /// * [`BoxOutOfOrder`](crate::DemuxError::BoxOutOfOrder),
    ///   [`DuplicateBox`](crate::DemuxError::DuplicateBox): what the
    ///   structure makes of a top-level box arriving where it does.
    /// * [`PayloadLimitExceeded`](crate::DemuxError::PayloadLimitExceeded):
    ///   a box read into a value reaches past the limit the demuxer gathers.
    /// * [`Sequence`](crate::DemuxError::Sequence): what the framing
    ///   of the file makes of the input.
    /// * [`Box`](crate::DemuxError::Box): a box read into a value
    ///   does not decode.
    /// * [`Sample`](crate::DemuxError::Sample): what the samples make
    ///   of the movie, a fragment, a `sidx`, or the media data beside them.
    /// * [`AlreadyFinished`](crate::DemuxError::AlreadyFinished): the
    ///   file was declared over, by [`finish`](Self::finish) or by
    ///   [`resume_at_movie_fragment_random_access`](Self::resume_at_movie_fragment_random_access).
    /// * [`UnwantedInput`](crate::DemuxError::UnwantedInput): `offset` is
    ///   not where [`wanted_input`](Self::wanted_input) names, nor, while the
    ///   input is taken in order, where it stands. The demuxer is not failed
    ///   by it.
    /// * The failure of a previous call, which the demuxer keeps and reports
    ///   again for every call after it.
    pub fn handle_input(&mut self, offset: u64, input: &[u8]) -> Result<(), DemuxError> {
        let State::LocatingMovieFragmentRandomAccess {
            file_len,
            mfro,
            filled,
        } = &mut self.state
        else {
            self.input.handle_input(offset, input)?;
            let read = self.read_framed();

            return self.input.record(read);
        };

        if input.is_empty() {
            return Ok(());
        }
        if offset != closing_offset(*file_len, *filled) {
            return Err(DemuxError::UnwantedInput {
                input_offset: offset,
            });
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
            None => {
                self.state = State::Reading;
                self.input.declare_over();
            }
        }

        Ok(())
    }

    /// Takes the next output the file handed over so far completed: a sample, or an extent
    ///
    /// Reports `None` once they are used up: more of the file is needed. Failure
    /// is reported by the calls that take it, so this one never fails — a failed
    /// demuxer hands over the output it had already completed, then `None`
    /// from there on.
    pub fn poll_output(&mut self) -> Option<G::Output> {
        self.input.poll_output()
    }

    /// Returns the one read wanted next, or `None` once the file is declared over or the demuxer has failed
    ///
    /// Gathering with [`SampleReader`], bytes the extent at the front of
    /// those held still lacks are wanted first, with their length, once the
    /// input taken in order has passed their start. Where the movie and each
    /// fragment precede the media data they address, a file handed over in
    /// order has none wanted; a movie whose chunks lie before it (§8.1.1 sets
    /// no order for an `mdat`), or a fragment addressing media data lying
    /// before it (§8.8.7 has a base data offset name any byte of the file),
    /// wants bytes already passed by. Gathering with
    /// [`SampleExtents`](crate::SampleExtents), none are ever
    /// lacked. Otherwise the continuation of the input taken in order is
    /// wanted, at the offset it stands at and with no length. While the bytes the file would close with an `mfro` in are gathered, after
    /// [`resume_at_movie_fragment_random_access`](Self::resume_at_movie_fragment_random_access),
    /// those still to gather are wanted alone, with their length. Bytes read
    /// for any of these are handed to [`handle_input`](Self::handle_input) at
    /// the offset they were read at.
    #[must_use]
    pub fn wanted_input(&self) -> Option<WantedInput> {
        match self.state {
            State::Reading => self.input.wanted_input(),
            State::LocatingMovieFragmentRandomAccess {
                file_len, filled, ..
            } => {
                let start = closing_offset(file_len, filled);

                Some(WantedInput::new(
                    start,
                    Some(file_len.saturating_sub(start)),
                ))
            }
        }
    }

    /// Returns the brands the file declares itself readable as, once they have arrived
    #[must_use]
    pub const fn file_type(&self) -> Option<&FileTypeBox> {
        self.file_type.as_ref()
    }

    /// Returns the movie of the file, once it has arrived
    #[must_use]
    pub const fn movie(&self) -> Option<&MovieBox> {
        self.movie.as_ref()
    }

    /// Returns the subsegments of every `sidx` read so far, in the order they were read
    ///
    /// A `sidx` read again, as the reading resumes at or before it, is held
    /// once. Each is placed in the file from the first byte after its `sidx`, as
    /// [`SegmentIndex::resolve`] places them.
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
    /// `mfra`. The demuxer resumes from reading and from the file declared
    /// over alike, and takes the file again from there.
    ///
    /// # Errors
    ///
    /// * The failure of a previous call, which the demuxer keeps and reports
    ///   again for every call after it.
    pub fn resume_at(&mut self, offset: u64) -> Result<(), DemuxError> {
        self.input.resumable()?;
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
    /// names is [`UnwantedInput`](crate::DemuxError::UnwantedInput), and
    /// [`finish`](Self::finish) declares the file over with no failure. The
    /// demuxer is told this from reading and from the file declared over
    /// alike; what [`resume_at`](Self::resume_at) drops is dropped only once
    /// the `mfra` is located.
    ///
    /// # Errors
    ///
    /// * The failure of a previous call, which the demuxer keeps and reports
    ///   again for every call after it.
    pub fn resume_at_movie_fragment_random_access(
        &mut self,
        file_len: u64,
    ) -> Result<(), DemuxError> {
        self.input.resumable()?;

        if file_len < MovieFragmentRandomAccessOffsetBox::ENCODED_LEN as u64 {
            self.state = State::Reading;
            self.input.declare_over();
        } else {
            self.state = State::LocatingMovieFragmentRandomAccess {
                file_len,
                mfro: [0; MovieFragmentRandomAccessOffsetBox::ENCODED_LEN],
                filled: 0,
            };
        }

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
    /// * [`Sequence`](crate::DemuxError::Sequence): the file ended
    ///   inside a box.
    /// * [`Box`](crate::DemuxError::Box): a box read into a value,
    ///   declaring no total, does not decode.
    /// * [`MissingMandatoryBox`](crate::DemuxError::MissingMandatoryBox):
    ///   the file carried no `moov`.
    /// * [`Sample`](crate::DemuxError::Sample): what the samples make
    ///   of the movie, a fragment or a `sidx` declaring no total, or a sample
    ///   the movie or a fragment declared is short of the data it claimed.
    /// * [`AlreadyFinished`](crate::DemuxError::AlreadyFinished): the
    ///   file was already declared over.
    /// * The failure of a previous call, which the demuxer keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<(), DemuxError> {
        if let State::LocatingMovieFragmentRandomAccess { .. } = self.state {
            self.state = State::Reading;
            self.input.declare_over();

            return Ok(());
        }
        self.input.finish_framing()?;
        let read = self.read_framed();
        self.input.record(read)?;
        let checked = self.structure.finish();

        self.input.finish(checked)
    }

    /// Restarts the reading at `offset`, whatever the demuxer stood at
    fn restart(&mut self, offset: u64) {
        self.input.restart(offset);
        self.structure.resume();
        self.decode_times = TrackDecodeTimes::unknown();
        self.open = None;
        self.state = State::Reading;
    }

    /// Reads every box the framing has finished framing so far
    fn read_framed(&mut self) -> Result<(), DemuxError> {
        while let Some((start, event)) = self.input.poll_event() {
            match event {
                BoxEvent::Header(header) => self
                    .structure
                    .handle_box_type(header.box_type())
                    .map_err(DemuxError::from)
                    .and_then(|disposition| {
                        self.open = match disposition {
                            FragmentedDisposition::FileType => Some(Open::FileType(
                                WholeBoxReader::begin(header, self.limits.payload())?,
                            )),
                            FragmentedDisposition::Movie => Some(Open::Movie(
                                WholeBoxReader::begin(header, self.limits.payload())?,
                            )),
                            FragmentedDisposition::MovieFragment => Some(Open::MovieFragment {
                                reader: WholeBoxReader::begin(header, self.limits.payload())?,
                                moof_start: start,
                            }),
                            FragmentedDisposition::SegmentIndex => Some(Open::SegmentIndex(
                                WholeBoxReader::begin(header, self.limits.payload())?,
                            )),
                            FragmentedDisposition::MovieFragmentRandomAccess => {
                                Some(Open::MovieFragmentRandomAccess(WholeBoxReader::begin(
                                    header,
                                    self.limits.payload(),
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
                    Some(Open::MediaData) => self.input.samples_mut().handle_input(start, &payload),
                    None => Ok(()),
                },
                BoxEvent::End => match self.open.take() {
                    Some(Open::FileType(reader)) => reader
                        .finish()
                        .map(|file_type| self.file_type = Some(file_type)),
                    Some(Open::Movie(reader)) => reader.finish().and_then(|movie| {
                        self.input.samples_mut().handle_sample_extents(
                            sample_table::sample_extents(&movie, self.limits.resolved_samples()),
                        )?;
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
                                return Err(DemuxError::BoxOutOfOrder {
                                    box_type: MovieFragmentBox::BOX_TYPE,
                                });
                            };

                            let extents = movie_fragment::sample_extents(
                                &movie_fragment,
                                movie,
                                moof_start,
                                &mut self.decode_times,
                                self.limits.resolved_samples(),
                            )?;
                            self.input.samples_mut().handle_sample_extents(extents)?;

                            Ok(())
                        })
                    }
                    Some(Open::SegmentIndex(reader)) => reader.finish().and_then(|sidx| {
                        let segment_index = SegmentIndex::resolve(&sidx, start)?;
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
            }?;
        }

        Ok(())
    }
}

/// Returns the file offset the bytes still to gather of the `mfro` closing a file `file_len` long begin at, `filled` of them gathered
const fn closing_offset(file_len: u64, filled: usize) -> u64 {
    let left = MovieFragmentRandomAccessOffsetBox::ENCODED_LEN.saturating_sub(filled);

    file_len.saturating_sub(left as u64)
}

impl Default for MovieDemuxer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
