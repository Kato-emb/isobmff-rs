//! [`NonFragmentedReader`], a non-fragmented movie file read as it arrives

use core::ops::Range;

use isobmff_boxes::{FileTypeBox, MovieBox};
use isobmff_sample::sample_table::sample_extents;
use isobmff_sample::{Sample, SampleReader};
use isobmff_sequence::{BoxEvent, BoxReader};

use super::NonFragmentedStructure;
use crate::{Disposition, StructureError, WholeBoxReader};

/// Reads the samples a non-fragmented movie file carries, taking it as it arrives
///
/// A non-fragmented movie file carries its samples in the sample tables of its
/// one movie (ISO/IEC 14496-12 §8.2.1) and their bytes in the media data
/// beside it, which the movie may lie before or after. This reader wires the
/// layers that read one: the framing of the file into boxes, the structure
/// that says what each top-level box is, the reading of the boxes it names
/// into values, the resolution of the sample tables of the movie into the
/// extents of its samples, and the gathering of those samples out of the
/// media data. It holds no rule of its own; a caller hands over bytes and
/// takes [`Sample`]s. It reaches for no source of its own: when to read and
/// from where stay with the caller.
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
///   have arrived: [`file_type`](Self::file_type) and [`movie`](Self::movie).
///   The media data is offered to the samples, and every other box is passed
///   over — a `moof` among them: the samples a fragment carries are not read
///   here, but by [`FragmentedReader`](crate::FragmentedReader).
/// * The order the boxes come in, and what a file that breaks it is reported
///   as, are the structure's: an `ftyp` after another box is
///   [`BoxOutOfOrder`](crate::StructureErrorKind::BoxOutOfOrder), a second
///   `moov` is [`DuplicateBox`](crate::StructureErrorKind::DuplicateBox), and
///   a file declared over without a `moov` is
///   [`MissingMandatoryBox`](crate::StructureErrorKind::MissingMandatoryBox).
///   A file carrying no `ftyp` reads all the same, as §4.3 allows.
/// * A box read into a value is gathered whole before it is read, so what it
///   declares is bounded — see [`with_limits`](Self::with_limits).
/// * The samples are read out of the media data once the movie has arrived,
///   and come out as their bytes arrive whole, as [`SampleReader`]'s contract
///   has it: a movie lying before its media data has every sample come out
///   in the order the file lays them down. Media data arriving before the
///   movie is dropped, since no sample has claimed it yet, so a movie lying
///   after its media data completes no sample by itself: from then on
///   [`wanted_extent`](Self::wanted_extent) names the bytes the sample held
///   longest still lacks, for a caller that can seek to fetch and hand to
///   [`handle_data`](Self::handle_data).
/// * An `Err` leaves the reader failed for good,
///   [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished) aside:
///   every later call reports that same failure again. The samples completed
///   before it are still there to take.
/// * [`finish`](Self::finish) declares the file over, and reports what any
///   layer makes of the end of it: a box left open, the `moov` never come, a
///   sample short of the data it claimed. Samples are still taken after it,
///   but anything handed over then, or a second [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished).
///
/// # Examples
///
/// ```
/// use isobmff::NonFragmentedReader;
/// # use isobmff_test_support::non_fragmented_file;
/// // A file of two chunks of one track, its movie lying after its media data
/// let file = non_fragmented_file(&[&[b"SAMP", b"DATA"], &[b"LAST"]], false);
///
/// // The file is handed over as it arrives: the media data comes before any
/// // sample has claimed it, so no sample is completed yet
/// let mut reader = NonFragmentedReader::new();
/// for arriving in file.chunks(7) {
///     reader.handle_input(arriving)?;
/// }
/// assert_eq!(reader.poll_sample(), None);
///
/// // The movie has arrived, and names the bytes its samples lack in turn
/// assert_eq!(reader.movie().map(|moov| moov.trak().len()), Some(1));
/// while let Some(wanted) = reader.wanted_extent() {
///     let fetched = &file[wanted.start as usize..wanted.end as usize];
///     reader.handle_data(wanted.start, fetched)?;
/// }
/// reader.finish()?;
///
/// // The samples come back as the file laid them down
/// let first = reader.poll_sample().unwrap();
/// assert_eq!((first.data(), first.decode_time()), (b"SAMP".as_slice(), 0));
/// let second = reader.poll_sample().unwrap();
/// assert_eq!((second.data(), second.decode_time()), (b"DATA".as_slice(), 3_000));
/// let third = reader.poll_sample().unwrap();
/// assert_eq!((third.data(), third.decode_time()), (b"LAST".as_slice(), 6_000));
/// assert_eq!(reader.poll_sample(), None);
/// # Ok::<(), isobmff::StructureError>(())
/// ```
#[derive(Debug)]
pub struct NonFragmentedReader {
    boxes: BoxReader,
    structure: NonFragmentedStructure,
    samples: SampleReader,
    open: Option<Open>,
    file_type: Option<FileTypeBox>,
    movie: Option<MovieBox>,
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
    Failed(StructureError),
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

impl NonFragmentedReader {
    /// Payload a box read into a value may declare, where the caller names no limit
    ///
    /// Sixteen mebibytes. A caller reading files whose `moov` reaches past that
    /// — a long presentation states a table row for every sample — names a
    /// limit of its own with [`with_limits`](Self::with_limits).
    pub const DEFAULT_PAYLOAD_LIMIT: u64 = 16 * 1024 * 1024;

    /// Creates a reader waiting at the start of a non-fragmented movie file
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
    /// [`PayloadLimitExceeded`](crate::StructureErrorKind::PayloadLimitExceeded)
    /// before a byte of it is gathered; a sample declaring more than
    /// `sample_size_limit` bytes is what
    /// [`SampleReader::with_sample_size_limit`](SampleReader::with_sample_size_limit)
    /// makes of it.
    #[must_use]
    pub const fn with_limits(payload_limit: u64, sample_size_limit: u64) -> Self {
        Self {
            boxes: BoxReader::new(),
            structure: NonFragmentedStructure::new(),
            samples: SampleReader::with_sample_size_limit(sample_size_limit),
            open: None,
            file_type: None,
            movie: None,
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
    /// * [`BoxOutOfOrder`](crate::StructureErrorKind::BoxOutOfOrder),
    ///   [`DuplicateBox`](crate::StructureErrorKind::DuplicateBox): what the
    ///   structure makes of a top-level box arriving where it does.
    /// * [`PayloadLimitExceeded`](crate::StructureErrorKind::PayloadLimitExceeded):
    ///   a box read into a value reaches past the limit the reader gathers.
    /// * [`Sequence`](crate::StructureErrorKind::Sequence): what the framing
    ///   of the file makes of the input.
    /// * [`Box`](crate::StructureErrorKind::Box): a box read into a value
    ///   does not decode.
    /// * [`Sample`](crate::StructureErrorKind::Sample): what the samples make
    ///   of the sample tables of the movie or the media data beside it.
    /// * [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn handle_input(&mut self, input: &[u8]) -> Result<(), StructureError> {
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
    /// from its first byte as a chunk offset is (ISO/IEC 14496-12 §8.7.5),
    /// and what they completed is then taken from
    /// [`poll_sample`](Self::poll_sample).
    /// The file handed over in order through
    /// [`handle_input`](Self::handle_input) goes on from where it stood.
    ///
    /// # Errors
    ///
    /// * [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn handle_data(&mut self, offset: u64, data: &[u8]) -> Result<(), StructureError> {
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

    /// Returns the bytes the earliest sample still held lacks, if any is held
    ///
    /// A movie lying before its media data names bytes still to arrive, which a
    /// caller handing the file over in order meets as they come; one lying
    /// after it names bytes already passed by, which a caller that can seek
    /// fetches and hands to [`handle_data`](Self::handle_data).
    #[must_use]
    pub fn wanted_extent(&self) -> Option<Range<u64>> {
        self.samples.wanted_extent()
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
    /// * [`Sequence`](crate::StructureErrorKind::Sequence): the file ended
    ///   inside a box.
    /// * [`Box`](crate::StructureErrorKind::Box): a box read into a value,
    ///   declaring no total, does not decode.
    /// * [`MissingMandatoryBox`](crate::StructureErrorKind::MissingMandatoryBox):
    ///   the file carried no `moov`.
    /// * [`Sample`](crate::StructureErrorKind::Sample): what the samples make
    ///   of a movie declaring no total, or a sample the movie declared is
    ///   short of the data it claimed — every sample of a movie lying after
    ///   its media data, unless the bytes it named were fetched.
    /// * [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished): the
    ///   file was already declared over.
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<(), StructureError> {
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
    const fn reading(&self) -> Result<(), StructureError> {
        match self.state {
            State::Reading => Ok(()),
            State::Finished => Err(StructureError::already_finished()),
            State::Failed(failure) => Err(failure),
        }
    }

    /// Reads every box the framing has finished framing so far
    fn read_framed(&mut self) -> Result<(), StructureError> {
        while let Some(event) = self.boxes.poll_event() {
            match event {
                BoxEvent::Header(header) => {
                    self.structure
                        .handle_box_type(header.box_type())
                        .and_then(|disposition| {
                            self.open = match disposition {
                                Disposition::FileType => Some(Open::FileType(
                                    WholeBoxReader::begin(header, self.payload_limit)?,
                                )),
                                Disposition::Movie => Some(Open::Movie(WholeBoxReader::begin(
                                    header,
                                    self.payload_limit,
                                )?)),
                                Disposition::MediaData => Some(Open::MediaData),
                                Disposition::Skip => None,
                                // Why not unreachable: the structure of a
                                // non-fragmented movie file never answers with
                                // a fragment, and `None` stands in place of a
                                // panic the lints forbid.
                                Disposition::MovieFragment => None,
                            };

                            Ok(())
                        })
                }
                BoxEvent::Payload(payload) => match &mut self.open {
                    Some(Open::FileType(reader)) => reader.handle_payload(payload),
                    Some(Open::Movie(reader)) => reader.handle_payload(payload),
                    Some(Open::MediaData) => {
                        // Why not unreachable: an event was taken, so the
                        // framing names the bytes it was read from, and the
                        // fallback is a degenerate position in place of a
                        // panic the lints forbid.
                        let start = self.boxes.event_extent().map_or(0, |extent| extent.start);

                        self.samples
                            .handle_data(start, &payload)
                            .map_err(StructureError::from)
                    }
                    None => Ok(()),
                },
                BoxEvent::End => match self.open.take() {
                    Some(Open::FileType(reader)) => reader
                        .finish()
                        .map(|file_type| self.file_type = Some(file_type)),
                    Some(Open::Movie(reader)) => reader.finish().and_then(|movie| {
                        for extent in sample_extents(&movie) {
                            self.samples.handle_sample_extent(extent?)?;
                        }
                        self.movie = Some(movie);

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
    const fn fail(&mut self, failure: StructureError) -> StructureError {
        self.state = State::Failed(failure);

        failure
    }
}

impl Default for NonFragmentedReader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use isobmff_boxes::{FileTypeBox, MovieBox};
    use isobmff_core::{BoxDefinition, BoxType};
    use isobmff_sample::{Sample, SampleReader};
    use isobmff_test_support::{
        file_type, framed, non_fragmented_file, unfragmented_movie, written,
    };

    use super::{NonFragmentedReader, StructureError};
    use crate::StructureErrorKind;

    /// What the reader makes of `file` handed over whole, then declared over
    fn read(file: &[u8]) -> Result<NonFragmentedReader, StructureError> {
        let mut reader = NonFragmentedReader::new();

        reader.handle_input(file)?;
        reader.finish()?;

        Ok(reader)
    }

    #[test]
    fn a_file_declaring_no_brands_is_read_all_the_same() {
        let reader = read(&written(&unfragmented_movie())).unwrap();

        assert_eq!(reader.file_type(), None);
        assert_eq!(reader.movie(), Some(&unfragmented_movie()));
    }

    #[test]
    fn a_file_declared_over_without_a_movie_is_rejected() {
        assert_eq!(
            read(&written(&file_type())).map(drop),
            Err(StructureError::missing_mandatory_box(MovieBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_box_read_into_a_value_declaring_a_payload_past_the_limit_is_rejected() {
        let mut reader =
            NonFragmentedReader::with_limits(4, SampleReader::DEFAULT_SAMPLE_SIZE_LIMIT);

        assert_eq!(
            reader
                .handle_input(&written(&file_type()))
                .map_err(StructureError::kind),
            Err(StructureErrorKind::PayloadLimitExceeded)
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
        let mut reader = NonFragmentedReader::with_limits(
            movie.len() as u64,
            SampleReader::DEFAULT_SAMPLE_SIZE_LIMIT,
        );

        reader.handle_input(&file).unwrap();

        assert_eq!(reader.finish(), Ok(()));
    }

    #[test]
    fn a_box_read_into_a_value_declaring_no_total_is_read_to_the_end_of_the_file() {
        let mut file = written(&unfragmented_movie());
        file.splice(..4, [0x00, 0x00, 0x00, 0x00]);

        let reader = read(&file).unwrap();

        assert_eq!(reader.movie(), Some(&unfragmented_movie()));
    }

    #[test]
    fn the_samples_completed_before_a_framing_failure_are_still_taken() {
        let mut file = non_fragmented_file(&[&[b"SAMP"]], true);
        file.extend_from_slice(b"\0\0\0\x04free");

        let mut reader = NonFragmentedReader::new();

        assert_eq!(
            reader.handle_input(&file).map_err(StructureError::kind),
            Err(StructureErrorKind::Sequence(
                isobmff_sequence::ErrorKind::Box(isobmff_core::ErrorKind::SizeBelowHeader)
            ))
        );
        assert_eq!(
            reader.poll_sample().map(Sample::into_data),
            Some(b"SAMP".to_vec())
        );
    }

    #[test]
    fn a_file_declared_over_with_a_sample_short_of_its_bytes_is_rejected() {
        let file = non_fragmented_file(&[&[b"SAMP"]], false);
        let mut reader = NonFragmentedReader::new();

        reader.handle_input(&file).unwrap();

        assert_eq!(
            reader.finish().map_err(StructureError::kind),
            Err(StructureErrorKind::Sample(
                isobmff_sample::SampleErrorKind::UnfinishedSample
            ))
        );
    }

    #[test]
    fn a_failed_reader_reports_the_same_failure_for_every_call_after_it() {
        let mut reader = NonFragmentedReader::new();
        let failure = StructureError::box_out_of_order(FileTypeBox::BOX_TYPE);
        let file = [written(&file_type()), written(&file_type())].concat();

        assert_eq!(reader.handle_input(&file), Err(failure));
        assert_eq!(
            reader.handle_input(&written(&unfragmented_movie())),
            Err(failure)
        );
        assert_eq!(reader.handle_data(0, b"SAMP"), Err(failure));
        assert_eq!(reader.finish(), Err(failure));
    }

    #[test]
    fn input_handed_over_after_finishing_is_rejected() {
        let mut reader = read(&written(&unfragmented_movie())).unwrap();

        assert_eq!(
            reader.handle_input(&written(&file_type())),
            Err(StructureError::already_finished())
        );
        assert_eq!(
            reader.handle_data(0, b"SAMP"),
            Err(StructureError::already_finished())
        );
        assert_eq!(reader.finish(), Err(StructureError::already_finished()));
    }
}
