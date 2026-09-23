//! [`FragmentedReader`], a fragmented movie file read as it arrives

use core::ops::Range;

use alloc::vec::Vec;

use isobmff_boxes::{
    FileTypeBox, MovieBox, MovieFragmentBox, MovieFragmentRandomAccessBox, SegmentIndexBox,
};
use isobmff_core::BoxDefinition;
use isobmff_sample::movie_fragment::sample_extents;
use isobmff_sample::segment_index::subsegments;
use isobmff_sample::{Sample, SampleReader, SegmentIndex, TrackDecodeTimes};
use isobmff_sequence::{BoxEvent, BoxReader};

use super::{FragmentedDisposition, FragmentedStructure};
use crate::{Error, WholeBoxReader};

/// Reads the samples a fragmented movie file carries, taking it as it arrives
///
/// A fragmented movie file is laid out as ISO/IEC 14496-12 Annex A.8 has it:
/// the brands it declares itself readable as, the movie its fragments
/// continue, then one movie fragment after another with the media data each
/// of them addresses. This reader wires the layers that read one: the framing
/// of the file into boxes, the structure that says what each top-level box
/// is, the reading of the boxes it names into values, the resolution of each
/// fragment against the movie into the extents of its samples, and the
/// gathering of those samples out of the media data. It holds no rule of its
/// own; a caller hands over bytes and takes [`Sample`]s. It reaches for no
/// source of its own: when to read and from where stay with the caller.
///
/// # Contract
///
/// * The file is handed over from its first byte, in order and cut anywhere,
///   and the samples it completed are taken from
///   [`poll_sample`](Self::poll_sample). The caller drains before handing
///   over more: samples are held until they are taken. Where the file lies in
///   its resource is the caller's: every offset the reader reports is a file
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
///   extents held and the samples not yet taken are dropped, and where each
///   track stands on its timeline is no longer known until a `tfdt` states
///   it.
/// * The order the boxes come in, and what a file that breaks it is reported
///   as, are the structure's: an `ftyp` after another box, a
///   `moof` before the `moov`, an `mdat` before any `moof` are
///   [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder), a second
///   `moov` is [`DuplicateBox`](crate::ErrorKind::DuplicateBox), and
///   a file declared over without a `moov` is
///   [`MissingMandatoryBox`](crate::ErrorKind::MissingMandatoryBox).
///   A file carrying no `ftyp` reads all the same, as §4.3 allows.
/// * A box read into a value is gathered whole before it is read, so what it
///   declares is bounded — see [`with_limits`](Self::with_limits).
/// * The samples of a fragment are read out of the media data that follows
///   it, and come out as their bytes arrive whole, as [`SampleReader`]'s
///   contract has it: the extents of a fragment are held in the order of
///   their bytes, so a file handed over in order yields the samples of each
///   fragment in the order they lie in it, whatever order the fragment
///   declares them in and wherever the input is cut.
///   [`wanted_extent`](Self::wanted_extent) names the bytes the extent at the
///   front of those held still lacks, which a caller handing the file over
///   in order meets as they come.
/// * An `Err` leaves the reader failed for good,
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished) aside:
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
/// use isobmff_boxes::{SampleFlags, TrackExtendsBox};
/// use isobmff_sample::Sample;
/// use isobmff_structure::{FragmentedReader, FragmentedWriter};
/// # use isobmff_test_support::{file_type, fragmented_movie};
/// // A file of one fragment carrying two samples of track 1
/// let mut writer = FragmentedWriter::new();
/// writer.handle_file_type(file_type())?;
/// writer.handle_movie(fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO)))?;
/// writer.begin_fragment(1)?;
/// writer.handle_sample(Sample::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec()))?;
/// writer.handle_sample(Sample::new(1, 1_024, 1_024, 0, SampleFlags::ZERO, 1, b"DATA".to_vec()))?;
/// writer.finish_fragment()?;
/// writer.finish()?;
///
/// // The file the writer laid down is drained as it hands the bytes over
/// let mut file = Vec::new();
/// while let Some(written) = writer.poll_output() {
///     file.extend_from_slice(&written);
/// }
///
/// // The file is handed over as it arrives, in whatever lengths it comes
/// let mut reader = FragmentedReader::new();
/// for arriving in file.chunks(7) {
///     reader.handle_input(arriving)?;
/// }
/// reader.finish()?;
///
/// // The brands and the movie the file declared are there to read
/// assert_eq!(reader.file_type().map(|ftyp| ftyp.major_brand()), Some(file_type().major_brand()));
/// assert_eq!(reader.movie().map(|moov| moov.trak().len()), Some(1));
///
/// // The samples come back as they were laid out
/// let first = reader.poll_sample().unwrap();
/// assert_eq!((first.data(), first.decode_time()), (b"SAMP".as_slice(), 0));
/// let second = reader.poll_sample().unwrap();
/// assert_eq!((second.data(), second.decode_time()), (b"DATA".as_slice(), 1_024));
/// assert_eq!(reader.poll_sample(), None);
/// # Ok::<(), isobmff_structure::Error>(())
/// ```
#[derive(Debug)]
pub struct FragmentedReader {
    boxes: BoxReader,
    base: u64,
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

/// Where the reader stands between calls
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

impl FragmentedReader {
    /// Payload a box read into a value may declare, where the caller names no limit
    ///
    /// Sixteen mebibytes. A caller reading files whose `moov` reaches past that
    /// — a presentation of many tracks states a sample entry for each — names a
    /// limit of its own with [`with_limits`](Self::with_limits).
    pub const DEFAULT_PAYLOAD_LIMIT: u64 = 16 * 1024 * 1024;

    /// Creates a reader waiting at the start of a fragmented movie file
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

    /// Creates a reader holding the file to `payload_limit` and `sample_size_limit`
    ///
    /// Both bound memory the reader is about to take, and both bound one box or
    /// one sample rather than the file. A box read into a value that declares
    /// more than `payload_limit` bytes of payload is
    /// [`PayloadLimitExceeded`](crate::ErrorKind::PayloadLimitExceeded)
    /// before a byte of it is gathered; a sample declaring more than
    /// `sample_size_limit` bytes is what
    /// [`SampleReader::with_sample_size_limit`](SampleReader::with_sample_size_limit)
    /// makes of it.
    #[must_use]
    pub const fn with_limits(payload_limit: u64, sample_size_limit: u64) -> Self {
        Self {
            boxes: BoxReader::new(),
            base: 0,
            structure: FragmentedStructure::new(),
            samples: SampleReader::with_sample_size_limit(sample_size_limit),
            decode_times: TrackDecodeTimes::new(),
            open: None,
            file_type: None,
            movie: None,
            segment_indexes: Vec::new(),
            movie_fragment_random_access: None,
            payload_limit,
            state: State::Reading,
        }
    }

    /// Takes the next cut of the file and reads the samples it completes
    ///
    /// The input is taken whole, as the continuation of what was handed over
    /// before it, the first cut starting at the first byte of the file. What
    /// the input completed is then taken from
    /// [`poll_sample`](Self::poll_sample).
    ///
    /// # Errors
    ///
    /// * [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder),
    ///   [`DuplicateBox`](crate::ErrorKind::DuplicateBox): what the
    ///   structure makes of a top-level box arriving where it does.
    /// * [`PayloadLimitExceeded`](crate::ErrorKind::PayloadLimitExceeded):
    ///   a box read into a value reaches past the limit the reader gathers.
    /// * [`Sequence`](crate::ErrorKind::Sequence): what the framing
    ///   of the file makes of the input.
    /// * [`Box`](crate::ErrorKind::Box): a box read into a value
    ///   does not decode.
    /// * [`Sample`](crate::ErrorKind::Sample): what the samples make
    ///   of a fragment or the media data beside it.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn handle_input(&mut self, input: &[u8]) -> Result<(), Error> {
        self.reading()?;

        // Why not failing before the events are read: the framing keeps the
        // events it made before failing, and the samples they complete are
        // the caller's to take, so they are read first and the failure kept
        // for after them.
        let framed = self.boxes.handle_input(input);
        self.read_framed()?;

        framed.map_err(|failure| self.fail(failure.into()))
    }

    /// Takes bytes of the file fetched for what [`wanted_extent`](Self::wanted_extent) named, and reads the samples they complete
    ///
    /// The bytes are offered to the samples alone, as the media data of the
    /// file is: `offset` is where the first of them lies in the file, counted
    /// from its first byte as a base data offset is (ISO/IEC 14496-12
    /// §8.8.7), and what they completed is then taken from
    /// [`poll_sample`](Self::poll_sample). The file handed over in order
    /// through [`handle_input`](Self::handle_input) goes on from where it
    /// stood.
    ///
    /// # Errors
    ///
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn handle_data(&mut self, offset: u64, data: &[u8]) -> Result<(), Error> {
        self.reading()?;
        self.samples
            .handle_data(offset, data)
            .map_err(|failure| self.fail(failure.into()))
    }

    /// Takes the next sample the file handed over so far completed
    ///
    /// Reports `None` once they are used up: more of the file is needed. Failure
    /// is reported by the calls that take it, so this one never fails — a failed
    /// reader hands over the samples it had already completed, then `None` from
    /// there on.
    pub fn poll_sample(&mut self) -> Option<Sample> {
        self.samples.poll_sample()
    }

    /// Returns the bytes the extent at the front of those held still lacks, if any is held
    ///
    /// A fragment precedes the media data it addresses, so a caller handing the
    /// file over in order meets every extent as it comes: what this names is
    /// media data still to arrive. A fragment addressing media data lying
    /// before it (§8.8.7 has a base data offset name any byte of the file)
    /// names bytes already passed by, which a caller that can seek fetches
    /// and hands to [`handle_data`](Self::handle_data).
    #[must_use]
    pub fn wanted_extent(&self) -> Option<Range<u64>> {
        self.samples.wanted_extent()
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
    /// A `sidx` read again, as the reading resumes at it a second time, is
    /// held once. Each is placed in the file from the first byte after its `sidx`, as
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
    /// `mfra`. The reader resumes from reading and from the file declared
    /// over alike, and takes the file again from there.
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
        self.base = offset;
        self.structure.resume();
        self.samples.clear();
        self.decode_times = TrackDecodeTimes::unknown();
        self.open = None;
        self.state = State::Reading;

        Ok(())
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
    ///   of a fragment declaring no total, or a sample a fragment declared is
    ///   short of the data it claimed.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was already declared over.
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
            // in place of a panic the lints forbid. Why not checked_add: the base
            // is where the framing restarted in the file, and the extent lies in
            // the bytes handed over past it, so both name one finite file.
            let start = self
                .boxes
                .event_extent()
                .map_or(0, |extent| extent.start)
                .saturating_add(self.base);
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
                    Some(Open::Movie(reader)) => {
                        reader.finish().map(|movie| self.movie = Some(movie))
                    }
                    Some(Open::MovieFragment { reader, moof_start }) => {
                        reader.finish().and_then(|movie_fragment| {
                            // Why not unreachable: the structure placed the `moof`
                            // after the `moov`, so the movie is there, and the
                            // fallback repeats what the structure answers a `moof`
                            // before it with, in place of a panic the lints forbid.
                            let Some(movie) = self.movie.as_ref() else {
                                return Err(Error::box_out_of_order(MovieFragmentBox::BOX_TYPE));
                            };

                            let extents = sample_extents(
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

    /// Fails the reader for good, and hands the failure back to report
    const fn fail(&mut self, failure: Error) -> Error {
        self.state = State::Failed(failure);

        failure
    }
}

impl Default for FragmentedReader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use isobmff_boxes::{FileTypeBox, MovieBox, SampleFlags, TrackExtendsBox};
    use isobmff_core::{BoxDefinition, BoxType};
    use isobmff_sample::{Sample, SampleReader};
    use isobmff_test_support::{file_type, fragmented_movie, framed, movie_fragment, written};

    use super::super::tests::{file_of_one_sample, sample};
    use super::{Error, FragmentedReader};
    use crate::ErrorKind;

    /// Movie of one track continued in fragments, whose defaults a `trex` states
    fn movie() -> MovieBox {
        fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO))
    }

    /// What the reader makes of `file` handed over whole, then declared over
    fn read(file: &[u8]) -> Result<FragmentedReader, Error> {
        let mut reader = FragmentedReader::new();

        reader.handle_input(file)?;
        reader.finish()?;

        Ok(reader)
    }

    #[test]
    fn a_file_declaring_no_brands_is_read_all_the_same() {
        let file = [written(&movie()), written(&movie_fragment())].concat();
        let reader = read(&file).unwrap();

        assert_eq!(reader.file_type(), None);
        assert_eq!(reader.movie(), Some(&movie()));
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
        let mut reader = FragmentedReader::with_limits(4, SampleReader::DEFAULT_SAMPLE_SIZE_LIMIT);

        assert_eq!(
            reader
                .handle_input(&written(&file_type()))
                .map_err(Error::kind),
            Err(ErrorKind::PayloadLimitExceeded)
        );
    }

    #[test]
    fn a_box_passed_over_is_not_bounded_by_the_limit() {
        let movie = written(&movie());
        let file = [
            movie.clone(),
            framed(BoxType::compact(*b"free"), &[0x11; 4_096]),
        ]
        .concat();
        let mut reader = FragmentedReader::with_limits(
            movie.len() as u64,
            SampleReader::DEFAULT_SAMPLE_SIZE_LIMIT,
        );

        reader.handle_input(&file).unwrap();

        assert_eq!(reader.finish(), Ok(()));
    }

    #[test]
    fn a_box_read_into_a_value_declaring_no_total_is_read_to_the_end_of_the_file() {
        let mut file = written(&movie());
        file.splice(..4, [0x00, 0x00, 0x00, 0x00]);

        let reader = read(&file).unwrap();

        assert_eq!(reader.movie(), Some(&movie()));
    }

    #[test]
    fn the_samples_completed_before_a_framing_failure_are_still_taken() {
        let mut file = file_of_one_sample();
        file.extend_from_slice(b"\0\0\0\x04free");

        let mut reader = FragmentedReader::new();

        assert_eq!(
            reader.handle_input(&file).map_err(Error::kind),
            Err(ErrorKind::Sequence(isobmff_sequence::ErrorKind::Box(
                isobmff_core::ErrorKind::SizeBelowHeader
            )))
        );
        assert_eq!(
            reader.poll_sample().map(Sample::into_data),
            Some(b"SAMP".to_vec())
        );
    }

    #[test]
    fn the_bytes_the_reader_wants_fetched_complete_the_sample_as_the_media_data_would() {
        let mut file = file_of_one_sample();
        let media_data = file.split_off(file.len().saturating_sub(4));

        let mut reader = FragmentedReader::new();
        reader.handle_input(&file).unwrap();
        let wanted = reader.wanted_extent().unwrap();
        reader.handle_data(wanted.start, &media_data).unwrap();

        let media_data_start = file.len() as u64;
        assert_eq!(wanted, media_data_start..media_data_start.saturating_add(4));
        assert_eq!(reader.poll_sample(), Some(sample()));
    }

    #[test]
    fn a_failed_reader_reports_the_same_failure_for_every_call_after_it() {
        let mut reader = FragmentedReader::new();
        let failure = Error::box_out_of_order(FileTypeBox::BOX_TYPE);
        let file = [written(&file_type()), written(&file_type())].concat();

        assert_eq!(reader.handle_input(&file), Err(failure));
        assert_eq!(reader.handle_input(&written(&movie())), Err(failure));
        assert_eq!(reader.handle_data(0, b"SAMP"), Err(failure));
        assert_eq!(reader.finish(), Err(failure));
    }

    #[test]
    fn input_handed_over_after_finishing_is_rejected() {
        let mut reader = read(&written(&movie())).unwrap();

        assert_eq!(
            reader.handle_input(&written(&file_type())),
            Err(Error::already_finished())
        );
        assert_eq!(
            reader.handle_data(0, b"SAMP"),
            Err(Error::already_finished())
        );
        assert_eq!(reader.finish(), Err(Error::already_finished()));
    }
}
