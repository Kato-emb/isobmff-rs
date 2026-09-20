//! [`FragmentedReader`], a fragmented movie file read as it arrives

use alloc::vec::Vec;
use core::ops::Range;

use isobmff_boxes::{FileTypeBox, MovieBox, MovieFragmentBox};
use isobmff_core::{BoxDefinition, BoxHeader};
use isobmff_sample::movie_fragment::sample_extents;
use isobmff_sample::{Sample, SampleReader, TrackDecodeTimes};
use isobmff_sequence::{BoxEvent, BoxReader};

use crate::{Disposition, FragmentedStructure, StructureError, WholeBoxReader};

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
/// * The file is handed over in order, cut anywhere, each cut with the offset
///   in the file it starts at, and the samples it completed are taken from
///   [`poll_sample`](Self::poll_sample). The caller drains before handing
///   over more: samples are held until they are taken.
/// * The boxes the structure reads into values are there to read once they
///   have arrived: [`file_type`](Self::file_type) and [`movie`](Self::movie).
///   The media data is offered to the samples, and every other box is passed
///   over.
/// * The order the boxes come in, and what a file that breaks it is reported
///   as, are the structure's: an `ftyp` after another box, a
///   `moof` before the `moov`, an `mdat` before any `moof` are
///   [`BoxOutOfOrder`](crate::StructureErrorKind::BoxOutOfOrder), a second
///   `moov` is [`DuplicateBox`](crate::StructureErrorKind::DuplicateBox), and
///   a file declared over without a `moov` is
///   [`MissingMandatoryBox`](crate::StructureErrorKind::MissingMandatoryBox).
///   A file carrying no `ftyp` reads all the same, as §4.3 allows.
/// * A box read into a value is gathered whole before it is read, so what it
///   declares is bounded — see [`with_limits`](Self::with_limits).
/// * The samples of a fragment are read in the order it declares them, out of
///   the media data that follows it. [`wanted_extent`](Self::wanted_extent)
///   names the bytes the earliest sample still lacks, which a caller handing
///   the file over in order meets as they come.
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
/// use isobmff::TrackExtendsBox;
/// use isobmff::fragmented_movie::{FragmentedReader, FragmentedWriter};
/// use isobmff_sample::Sample;
/// # use isobmff_test_support::{file_type, fragmented_movie};
/// // A file of one fragment carrying two samples of track 1
/// let mut writer = FragmentedWriter::new();
/// writer.handle_file_type(file_type())?;
/// writer.handle_movie(fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, 0)))?;
/// writer.begin_fragment(1)?;
/// writer.handle_sample(Sample::new(1, 0, 1_024, 0, 0, 1, b"SAMP".to_vec()))?;
/// writer.handle_sample(Sample::new(1, 1_024, 1_024, 0, 0, 1, b"DATA".to_vec()))?;
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
/// let mut offset = 0;
/// for arriving in file.chunks(7) {
///     reader.handle_input(offset, arriving)?;
///     offset += arriving.len() as u64;
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
/// # Ok::<(), isobmff::StructureError>(())
/// ```
#[derive(Debug)]
pub struct FragmentedReader {
    boxes: BoxReader,
    structure: FragmentedStructure,
    samples: SampleReader,
    decode_times: TrackDecodeTimes,
    open: Option<Open>,
    file_type: Option<FileTypeBox>,
    movie: Option<MovieBox>,
    payload_limit: u64,
    handed: u64,
    origin: u64,
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
    /// Fragment being read whole, and where in the file it began
    MovieFragment {
        reader: WholeBoxReader<MovieFragmentBox>,
        moof_start: u64,
    },
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
    /// [`PayloadLimitExceeded`](crate::StructureErrorKind::PayloadLimitExceeded)
    /// before a byte of it is gathered; a sample declaring more than
    /// `sample_size_limit` bytes is what
    /// [`SampleReader::with_sample_size_limit`](SampleReader::with_sample_size_limit)
    /// makes of it.
    #[must_use]
    pub const fn with_limits(payload_limit: u64, sample_size_limit: u64) -> Self {
        Self {
            boxes: BoxReader::new(),
            structure: FragmentedStructure::new(),
            samples: SampleReader::with_sample_size_limit(sample_size_limit),
            decode_times: TrackDecodeTimes::new(),
            open: None,
            file_type: None,
            movie: None,
            payload_limit,
            handed: 0,
            origin: 0,
            state: State::Reading,
        }
    }

    /// Takes the next cut of the file, lying at `offset` in it, and reads the samples it completes
    ///
    /// The input is taken whole, as the continuation of what was handed over
    /// before it; `offset` is where its first byte lies in the file, which is
    /// what the extents of the samples are resolved against. What the input
    /// completed is then taken from [`poll_sample`](Self::poll_sample).
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
    ///   of a fragment or the media data beside it.
    /// * [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn handle_input(&mut self, offset: u64, input: &[u8]) -> Result<(), StructureError> {
        self.reading()?;

        // Why not checked arithmetic: a caller handing the file over in order
        // keeps `offset` at the bytes handed over before, plus where the file
        // begins in its resource, so the difference cannot go below zero, and
        // it read the bytes out of a finite resource, so the sum cannot run
        // past what 64 bits carry.
        self.origin = offset.saturating_sub(self.handed);
        self.handed = self.handed.saturating_add(input.len() as u64);

        self.boxes
            .handle_input(input)
            .map_err(|failure| self.fail(failure.into()))?;

        self.read_framed()
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
    /// A fragment precedes the media data it addresses, so a caller handing the
    /// file over in order meets every extent as it comes: what this names is
    /// media data still to arrive.
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
    ///   of a fragment declaring no total, or a sample a fragment declared is
    ///   short of the data it claimed.
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
            // Why not unreachable: an event was taken, so the framing names the
            // bytes it was read from, and the fallback is a degenerate range in
            // place of a panic the lints forbid.
            let extent = self.boxes.event_extent().unwrap_or(0..0);
            let extent =
                extent.start.saturating_add(self.origin)..extent.end.saturating_add(self.origin);
            let outcome = match event {
                BoxEvent::Header(header) => self.begin_box(header, extent.start),
                BoxEvent::Payload(payload) => self.take_payload(payload, extent.start),
                BoxEvent::End => self.close_box(),
                // Why an arm at all: `BoxEvent` is `#[non_exhaustive]`, which
                // `clippy::exhaustive_enums` asks of every public enum, so §4.2
                // being settled at three steps does not close the match.
                _later_step => Ok(()),
            };

            if let Err(failure) = outcome {
                return Err(self.fail(failure));
            }
        }

        Ok(())
    }

    /// Opens the box `header` introduces, as the structure disposes of it
    fn begin_box(&mut self, header: BoxHeader, start: u64) -> Result<(), StructureError> {
        self.open = match self.structure.handle_header(header)? {
            Disposition::FileType => Some(Open::FileType(WholeBoxReader::begin(
                header,
                self.payload_limit,
            )?)),
            Disposition::Movie => Some(Open::Movie(WholeBoxReader::begin(
                header,
                self.payload_limit,
            )?)),
            Disposition::MovieFragment => Some(Open::MovieFragment {
                reader: WholeBoxReader::begin(header, self.payload_limit)?,
                moof_start: start,
            }),
            Disposition::MediaData => Some(Open::MediaData),
            Disposition::Skip => None,
        };

        Ok(())
    }

    /// Gathers `payload` into the box that is open, or offers it to the samples
    fn take_payload(&mut self, payload: Vec<u8>, start: u64) -> Result<(), StructureError> {
        match &mut self.open {
            Some(Open::FileType(reader)) => reader.handle_payload(payload),
            Some(Open::Movie(reader)) => reader.handle_payload(payload),
            Some(Open::MovieFragment { reader, .. }) => reader.handle_payload(payload),
            Some(Open::MediaData) => Ok(self.samples.handle_data(start, &payload)?),
            None => Ok(()),
        }
    }

    /// Reads the box that ended into its value, and resolves a fragment against the movie
    fn close_box(&mut self) -> Result<(), StructureError> {
        match self.open.take() {
            Some(Open::FileType(reader)) => self.file_type = Some(reader.finish()?),
            Some(Open::Movie(reader)) => self.movie = Some(reader.finish()?),
            Some(Open::MovieFragment { reader, moof_start }) => {
                let movie_fragment = reader.finish()?;
                // Why not unreachable: the structure placed the `moof` after the
                // `moov`, so the movie is there, and the fallback repeats what
                // the structure answers a `moof` before it with, in place of a
                // panic the lints forbid.
                let Some(movie) = self.movie.as_ref() else {
                    return Err(StructureError::box_out_of_order(MovieFragmentBox::BOX_TYPE));
                };

                let extents =
                    sample_extents(&movie_fragment, movie, moof_start, &mut self.decode_times)?;
                for extent in extents {
                    self.samples.handle_sample_extent(extent?)?;
                }
            }
            Some(Open::MediaData) | None => {}
        }

        Ok(())
    }

    /// Fails the reader for good, and hands the failure back to report
    const fn fail(&mut self, failure: StructureError) -> StructureError {
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
    use isobmff_boxes::{FileTypeBox, MovieBox, TrackExtendsBox};
    use isobmff_core::{BoxDefinition, BoxType};
    use isobmff_sample::SampleReader;
    use isobmff_test_support::{file_type, fragmented_movie, framed, movie_fragment, written};

    use super::{FragmentedReader, StructureError};
    use crate::StructureErrorKind;

    /// Movie of one track continued in fragments, whose defaults a `trex` states
    fn movie() -> MovieBox {
        fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, 0))
    }

    /// What the reader makes of `file` handed over whole, then declared over
    fn read(file: &[u8]) -> Result<FragmentedReader, StructureError> {
        let mut reader = FragmentedReader::new();

        reader.handle_input(0, file)?;
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
            Err(StructureError::missing_mandatory_box(MovieBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_box_read_into_a_value_declaring_a_payload_past_the_limit_is_rejected() {
        let mut reader = FragmentedReader::with_limits(4, SampleReader::DEFAULT_SAMPLE_SIZE_LIMIT);

        assert_eq!(
            reader
                .handle_input(0, &written(&file_type()))
                .map_err(StructureError::kind),
            Err(StructureErrorKind::PayloadLimitExceeded)
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

        reader.handle_input(0, &file).unwrap();

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
    fn a_failed_reader_reports_the_same_failure_for_every_call_after_it() {
        let mut reader = FragmentedReader::new();
        let failure = StructureError::box_out_of_order(FileTypeBox::BOX_TYPE);
        let file = [written(&file_type()), written(&file_type())].concat();

        assert_eq!(reader.handle_input(0, &file), Err(failure));
        assert_eq!(reader.handle_input(0, &written(&movie())), Err(failure));
        assert_eq!(reader.finish(), Err(failure));
    }

    #[test]
    fn input_handed_over_after_finishing_is_rejected() {
        let mut reader = read(&written(&movie())).unwrap();

        assert_eq!(
            reader.handle_input(0, &written(&file_type())),
            Err(StructureError::already_finished())
        );
        assert_eq!(reader.finish(), Err(StructureError::already_finished()));
    }
}
