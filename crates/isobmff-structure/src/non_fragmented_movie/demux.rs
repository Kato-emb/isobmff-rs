//! [`NonFragmentedDemuxFsm`], a non-fragmented movie file read as it arrives

use isobmff_boxes::{FileTypeBox, MovieBox};
use isobmff_sample::sample_table::sample_extents;
use isobmff_sample::{Sample, SampleReader};
use isobmff_sequence::{BoxEvent, BoxReader};

use super::{NonFragmentedDisposition, NonFragmentedStructure};
use crate::{Error, InputPosition, InputRoute, WantedInput, WholeBoxReader};

/// Reads the samples a non-fragmented movie file carries, taking it as it arrives
///
/// A non-fragmented movie file carries its samples in the sample tables of its
/// one movie (ISO/IEC 14496-12 §8.2.1) and their bytes in the media data
/// beside it, which the movie may lie before or after. This demux FSM wires the
/// layers that read one: the framing of the file into boxes, the structure
/// that says what each top-level box is, the reading of the boxes it names
/// into values, the resolution of the sample tables of the movie into the
/// extents of its samples, and the gathering of those samples out of the
/// media data. It holds no rule of its own but one: of the bytes the samples
/// still lack, it names only those whose start the file handed over in order
/// has passed. A caller hands over bytes and takes [`Sample`]s. It reaches for
/// no source of its own: when to read and from where stay with the caller.
///
/// # Contract
///
/// * The file is handed over from its first byte, in order and cut anywhere,
///   each cut handed over at the offset it was read at, where
///   [`wanted_input`](Self::wanted_input) names,
///   and the samples it completed are taken from
///   [`poll_sample`](Self::poll_sample). The caller drains before handing
///   over more: samples are held until they are taken. Where the file lies in
///   its resource is the caller's: every offset the demux FSM reports is a file
///   offset, counting from the first byte of the file as the boxes count
///   theirs (§8.7.5, §8.8.7).
/// * The boxes the structure reads into values are there to read once they
///   have arrived: [`file_type`](Self::file_type) and [`movie`](Self::movie).
///   The media data is offered to the samples, and every other box is passed
///   over — a `moof` among them: the samples a fragment carries are not read
///   here, but by [`FragmentedDemuxFsm`](crate::FragmentedDemuxFsm).
/// * The order the boxes come in, and what a file that breaks it is reported
///   as, are the structure's: an `ftyp` after another box is
///   [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder), a second
///   `moov` is [`DuplicateBox`](crate::ErrorKind::DuplicateBox), and
///   a file declared over without a `moov` is
///   [`MissingMandatoryBox`](crate::ErrorKind::MissingMandatoryBox).
///   A file carrying no `ftyp` reads all the same, as §4.3 allows.
/// * A box read into a value is gathered whole before it is read, so what it
///   declares is bounded — see [`with_limits`](Self::with_limits).
/// * The samples are read out of the media data once the movie has arrived,
///   and come out as their bytes arrive whole, as [`SampleReader`]'s contract
///   has it: a movie lying before its media data has every sample come out
///   in the order the file lays them down. Media data arriving before the
///   movie is dropped, since no sample has claimed it yet, so a movie lying
///   after its media data completes no sample by itself: from then on
///   [`wanted_input`](Self::wanted_input) names, once the input has passed
///   its start, the bytes the extent at the front of those held still lacks,
///   for a caller that can seek to fetch and hand to
///   [`handle_input`](Self::handle_input) at their offset.
/// * An `Err` leaves the demux FSM failed for good,
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished) and
///   [`UnwantedInput`](crate::ErrorKind::UnwantedInput) aside:
///   every later call reports that same failure again. The samples completed
///   before it are still there to take.
/// * [`finish`](Self::finish) declares the file over, and reports what any
///   layer makes of the end of it: a box left open, the `moov` never come, a
///   sample short of the data it claimed. Samples are still taken after it,
///   but anything handed over then, or a second [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished).
///
/// # Examples
///
/// ```
/// use isobmff_structure::NonFragmentedDemuxFsm;
/// # use isobmff_test_support::non_fragmented_file;
/// // A file of two chunks of one track, its movie lying after its media data
/// let file = non_fragmented_file(&[&[b"SAMP", b"DATA"], &[b"LAST"]], false);
///
/// // The file is read where the demux FSM wants it, seven bytes at a time where it names no length
/// let mut demux_fsm = NonFragmentedDemuxFsm::new();
/// while let Some(wanted) = demux_fsm.wanted_input() {
///     let start = (wanted.offset() as usize).min(file.len());
///     let end = wanted.length().map_or(start + 7, |length| start + length as usize);
///     let read = &file[start..end.min(file.len())];
///     if read.is_empty() {
///         demux_fsm.finish()?;
///     } else {
///         demux_fsm.handle_input(wanted.offset(), read)?;
///     }
/// }
/// assert_eq!(demux_fsm.movie().map(|moov| moov.trak().len()), Some(1));
///
/// // The samples come back as the file laid them down
/// let first = demux_fsm.poll_sample().unwrap();
/// assert_eq!((first.data(), first.decode_time()), (b"SAMP".as_slice(), 0));
/// let second = demux_fsm.poll_sample().unwrap();
/// assert_eq!((second.data(), second.decode_time()), (b"DATA".as_slice(), 3_000));
/// let third = demux_fsm.poll_sample().unwrap();
/// assert_eq!((third.data(), third.decode_time()), (b"LAST".as_slice(), 6_000));
/// assert_eq!(demux_fsm.poll_sample(), None);
/// # Ok::<(), isobmff_structure::Error>(())
/// ```
#[derive(Debug)]
pub struct NonFragmentedDemuxFsm {
    boxes: BoxReader,
    position: InputPosition,
    structure: NonFragmentedStructure,
    samples: SampleReader,
    open: Option<Open>,
    file_type: Option<FileTypeBox>,
    movie: Option<MovieBox>,
    payload_limit: u64,
    state: State,
}

/// Where the demux FSM stands between calls
#[derive(Clone, Copy, Debug)]
enum State {
    /// Taking the file as it arrives
    Reading,
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
    /// Media data, offered to the samples as it arrives
    MediaData,
}

impl NonFragmentedDemuxFsm {
    /// Payload a box read into a value may declare, where the caller names no limit
    ///
    /// Sixteen mebibytes. A caller reading files whose `moov` reaches past that
    /// — a long presentation states a table row for every sample — names a
    /// limit of its own with [`with_limits`](Self::with_limits).
    pub const DEFAULT_PAYLOAD_LIMIT: u64 = 16 * 1024 * 1024;

    /// Creates a demux FSM waiting at the start of a non-fragmented movie file
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
            structure: NonFragmentedStructure::new(),
            samples: SampleReader::with_sample_size_limit(sample_size_limit),
            open: None,
            file_type: None,
            movie: None,
            payload_limit,
            state: State::Reading,
        }
    }

    /// Takes bytes of the file read at `offset`, and reads the samples they complete
    ///
    /// `offset` is a file offset, counted from the first byte of the file as a
    /// chunk offset is (ISO/IEC 14496-12 §8.7.5). Bytes at the offset the input
    /// taken in order stands at — the first byte of the file, then where the
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
    ///   of the sample tables of the movie or the media data beside it.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * [`UnwantedInput`](crate::ErrorKind::UnwantedInput): `offset` is
    ///   neither where the input taken in order stands nor the offset of the
    ///   bytes [`wanted_input`](Self::wanted_input) names as lacking. The
    ///   demux FSM is not failed by it.
    /// * The failure of a previous call, which the demux FSM keeps and reports
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
    /// start. A movie lying before its media data has none wanted; one lying
    /// after it wants bytes already passed by. Otherwise the continuation of
    /// the input taken in order is wanted, at the offset it stands at and with
    /// no length. Bytes read for either are handed to
    /// [`handle_input`](Self::handle_input) at the offset they were read at.
    #[must_use]
    pub fn wanted_input(&self) -> Option<WantedInput> {
        matches!(self.state, State::Reading)
            .then(|| self.position.wanted_input(self.samples.wanted_extent()))
    }

    /// Returns the brands the file declares itself readable as, once they have arrived
    #[must_use]
    pub const fn file_type(&self) -> Option<&FileTypeBox> {
        self.file_type.as_ref()
    }

    /// Returns the movie whose sample tables declare the samples of the file, once it has arrived
    #[must_use]
    pub const fn movie(&self) -> Option<&MovieBox> {
        self.movie.as_ref()
    }

    /// Declares the file over
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
    ///   of a movie declaring no total, or a sample the movie declared is
    ///   short of the data it claimed — every sample of a movie lying after
    ///   its media data, unless the bytes it named were fetched.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was already declared over.
    /// * The failure of a previous call, which the demux FSM keeps and reports
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

    /// Returns `Ok` while the demux FSM still takes what arrives
    const fn reading(&self) -> Result<(), Error> {
        match self.state {
            State::Reading => Ok(()),
            State::Finished => Err(Error::already_finished()),
            State::Failed(failure) => Err(failure),
        }
    }

    /// Reads every box the framing has finished framing so far
    fn read_framed(&mut self) -> Result<(), Error> {
        while let Some((extent, event)) = self.boxes.poll_event() {
            match event {
                BoxEvent::Header(header) => self
                    .structure
                    .handle_box_type(header.box_type())
                    .and_then(|disposition| {
                        self.open = match disposition {
                            NonFragmentedDisposition::FileType => Some(Open::FileType(
                                WholeBoxReader::begin(header, self.payload_limit)?,
                            )),
                            NonFragmentedDisposition::Movie => Some(Open::Movie(
                                WholeBoxReader::begin(header, self.payload_limit)?,
                            )),
                            NonFragmentedDisposition::MediaData => Some(Open::MediaData),
                            NonFragmentedDisposition::Skip => None,
                        };

                        Ok(())
                    }),
                BoxEvent::Payload(payload) => match &mut self.open {
                    Some(Open::FileType(reader)) => reader.handle_payload(payload),
                    Some(Open::Movie(reader)) => reader.handle_payload(payload),
                    Some(Open::MediaData) => self
                        .samples
                        .handle_data(extent.start, &payload)
                        .map_err(Error::from),
                    None => Ok(()),
                },
                BoxEvent::End => match self.open.take() {
                    Some(Open::FileType(reader)) => reader
                        .finish()
                        .map(|file_type| self.file_type = Some(file_type)),
                    Some(Open::Movie(reader)) => reader.finish().and_then(|movie| {
                        self.samples.handle_sample_extents(sample_extents(&movie))?;
                        self.movie = Some(movie);

                        Ok(())
                    }),
                    Some(Open::MediaData) | None => Ok(()),
                },
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

impl Default for NonFragmentedDemuxFsm {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use isobmff_boxes::{FileTypeBox, MovieBox};
    use isobmff_core::{BoxDefinition, BoxType};
    use isobmff_sample::{Sample, SampleReader};
    use isobmff_test_support::{
        file_type, framed, non_fragmented_file, unfragmented_movie, written,
    };

    use super::{Error, NonFragmentedDemuxFsm};
    use crate::ErrorKind;
    use crate::WantedInput;

    /// What the demux FSM makes of `file` handed over whole, then declared over
    fn read(file: &[u8]) -> Result<NonFragmentedDemuxFsm, Error> {
        let mut demux_fsm = NonFragmentedDemuxFsm::new();

        demux_fsm.handle_input(0, file)?;
        demux_fsm.finish()?;

        Ok(demux_fsm)
    }

    #[test]
    fn a_file_declaring_no_brands_is_read_all_the_same() {
        let demux_fsm = read(&written(&unfragmented_movie())).unwrap();

        assert_eq!(demux_fsm.file_type(), None);
        assert_eq!(demux_fsm.movie(), Some(&unfragmented_movie()));
    }

    #[test]
    fn a_file_declared_over_without_a_movie_is_rejected() {
        assert_eq!(
            read(&written(&file_type())).map(drop),
            Err(Error::missing_mandatory_box(MovieBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_box_read_into_a_value_declaring_a_payload_past_the_limit_is_rejected() {
        let mut demux_fsm =
            NonFragmentedDemuxFsm::with_limits(4, SampleReader::DEFAULT_SAMPLE_SIZE_LIMIT);

        assert_eq!(
            demux_fsm
                .handle_input(0, &written(&file_type()))
                .map_err(Error::kind),
            Err(ErrorKind::PayloadLimitExceeded)
        );
    }

    #[test]
    fn a_box_passed_over_is_not_bounded_by_the_limit() {
        let movie = written(&unfragmented_movie());
        let file = [
            movie.clone(),
            framed(BoxType::compact(*b"free"), &[0x11; 4_096]),
        ]
        .concat();
        let mut demux_fsm = NonFragmentedDemuxFsm::with_limits(
            movie.len() as u64,
            SampleReader::DEFAULT_SAMPLE_SIZE_LIMIT,
        );

        demux_fsm.handle_input(0, &file).unwrap();

        assert_eq!(demux_fsm.finish(), Ok(()));
    }

    #[test]
    fn a_box_read_into_a_value_declaring_no_total_is_read_to_the_end_of_the_file() {
        let mut file = written(&unfragmented_movie());
        file.splice(..4, [0x00, 0x00, 0x00, 0x00]);

        let demux_fsm = read(&file).unwrap();

        assert_eq!(demux_fsm.movie(), Some(&unfragmented_movie()));
    }

    #[test]
    fn the_samples_completed_before_a_framing_failure_are_still_taken() {
        let mut file = non_fragmented_file(&[&[b"SAMP"]], true);
        file.extend_from_slice(b"\0\0\0\x04free");

        let mut demux_fsm = NonFragmentedDemuxFsm::new();

        assert_eq!(
            demux_fsm.handle_input(0, &file).map_err(Error::kind),
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
    fn a_file_declared_over_with_a_sample_short_of_its_bytes_is_rejected() {
        let file = non_fragmented_file(&[&[b"SAMP"]], false);
        let mut demux_fsm = NonFragmentedDemuxFsm::new();

        demux_fsm.handle_input(0, &file).unwrap();

        assert_eq!(
            demux_fsm.finish().map_err(Error::kind),
            Err(ErrorKind::Sample(
                isobmff_sample::ErrorKind::UnfinishedSample
            ))
        );
    }

    #[test]
    fn a_failed_demux_fsm_reports_the_same_failure_for_every_call_after_it() {
        let mut demux_fsm = NonFragmentedDemuxFsm::new();
        let failure = Error::box_out_of_order(FileTypeBox::BOX_TYPE);
        let file = [written(&file_type()), written(&file_type())].concat();

        assert_eq!(demux_fsm.handle_input(0, &file), Err(failure));
        assert_eq!(
            demux_fsm.handle_input(0, &written(&unfragmented_movie())),
            Err(failure)
        );
        assert_eq!(demux_fsm.finish(), Err(failure));
    }

    #[test]
    fn input_handed_over_after_finishing_is_rejected() {
        let mut demux_fsm = read(&written(&unfragmented_movie())).unwrap();

        assert_eq!(
            demux_fsm.handle_input(0, &written(&file_type())),
            Err(Error::already_finished())
        );
        assert_eq!(demux_fsm.finish(), Err(Error::already_finished()));
    }

    /// The demux FSM handed whole a file whose movie lies after its media data, that file, and the offset of its one sample
    fn movie_after_its_media_data() -> (NonFragmentedDemuxFsm, Vec<u8>, u64) {
        let file = non_fragmented_file(&[&[b"SAMP"]], false);
        let lacking = file.windows(4).position(|bytes| bytes == b"SAMP").unwrap() as u64;
        let mut demux_fsm = NonFragmentedDemuxFsm::new();

        demux_fsm.handle_input(0, &file).unwrap();

        (demux_fsm, file, lacking)
    }

    #[test]
    fn bytes_a_movie_lying_after_its_media_data_lacks_are_wanted_with_their_length() {
        let (demux_fsm, _, lacking) = movie_after_its_media_data();

        assert_eq!(
            demux_fsm.wanted_input(),
            Some(WantedInput::new(lacking, Some(4)))
        );
    }

    #[test]
    fn input_at_the_offset_wanted_completes_the_sample_and_the_continuation_is_wanted_after_it() {
        let (mut demux_fsm, file, lacking) = movie_after_its_media_data();

        demux_fsm.handle_input(lacking, b"SAMP").unwrap();

        assert_eq!(
            demux_fsm.poll_sample().map(Sample::into_data),
            Some(b"SAMP".to_vec())
        );
        assert_eq!(
            demux_fsm.wanted_input(),
            Some(WantedInput::new(file.len() as u64, None))
        );
    }

    #[test]
    fn input_in_order_is_taken_while_bytes_are_wanted() {
        let (mut demux_fsm, file, lacking) = movie_after_its_media_data();

        demux_fsm
            .handle_input(file.len() as u64, &framed(BoxType::compact(*b"free"), &[]))
            .unwrap();

        assert_eq!(
            demux_fsm.wanted_input(),
            Some(WantedInput::new(lacking, Some(4)))
        );
    }

    #[test]
    fn input_at_an_offset_neither_in_order_nor_wanted_is_refused_and_the_file_reads_on() {
        let (mut demux_fsm, _, lacking) = movie_after_its_media_data();

        assert_eq!(
            demux_fsm.handle_input(lacking + 1, b"AMP"),
            Err(Error::unwanted_input(lacking + 1))
        );
        assert_eq!(demux_fsm.handle_input(lacking, b"SAMP"), Ok(()));
        assert_eq!(demux_fsm.finish(), Ok(()));
    }

    #[test]
    fn empty_input_is_taken_as_nothing_wherever_it_is_handed_over() {
        let mut demux_fsm = NonFragmentedDemuxFsm::new();

        assert_eq!(demux_fsm.handle_input(9, &[]), Ok(()));
        assert_eq!(demux_fsm.wanted_input(), Some(WantedInput::new(0, None)));
    }

    #[test]
    fn nothing_is_wanted_once_the_file_is_declared_over() {
        let demux_fsm = read(&non_fragmented_file(&[&[b"SAMP"]], true)).unwrap();

        assert_eq!(demux_fsm.wanted_input(), None);
    }
}
