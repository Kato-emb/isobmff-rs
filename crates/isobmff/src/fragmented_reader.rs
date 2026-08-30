//! [`FragmentedReader`], a fragmented movie file read as it arrives

use alloc::vec::Vec;
use core::ops::Range;

use isobmff_boxes::{FileTypeBox, MovieBox, MovieFragmentBox};
use isobmff_core::{BoxDecode, BoxDefinition, BoxHeader, BoxType};
use isobmff_sequence::{BoxEvent, BoxReader};

use crate::{FileError, Sample, SampleReader};

/// Box the layout of a fragmented file is made of, and this layer reads into a value
#[derive(Clone, Copy, Debug)]
enum ValueBox {
    /// [`FileTypeBox`] (`ftyp`)
    FileType,
    /// [`MovieBox`] (`moov`)
    Movie,
    /// [`MovieFragmentBox`] (`moof`)
    MovieFragment,
}

impl ValueBox {
    /// Returns the box `box_type` names, when it is one this layout is made of
    fn of(box_type: BoxType) -> Option<Self> {
        if box_type == FileTypeBox::BOX_TYPE {
            Some(Self::FileType)
        } else if box_type == MovieBox::BOX_TYPE {
            Some(Self::Movie)
        } else if box_type == MovieFragmentBox::BOX_TYPE {
            Some(Self::MovieFragment)
        } else {
            None
        }
    }
}

/// Payload of a box being read into a value, gathered as the input cut it
#[derive(Debug)]
struct Gathering {
    value_box: ValueBox,
    began_at: u64,
    payload: Vec<u8>,
}

/// Where the reader stands between calls
#[derive(Clone, Copy, Debug)]
enum State {
    /// Taking the file as it arrives
    Reading,
    /// Told the file is over, and taking no more input
    Finished,
    /// Failed, and reporting that same failure for every call after it
    Failed(FileError),
}

/// Reads the samples a fragmented movie file carries, taking it as it arrives
///
/// A fragmented movie file is laid out as ISO/IEC 14496-12 Annex A.8 has it: the
/// brands it declares itself readable as, the movie its fragments continue, then
/// one movie fragment after another with the media data each of them addresses.
/// This reader holds that layout. It frames the file into boxes, reads the ones
/// the layout is made of into values, and hands the fragments and the media data
/// to a [`SampleReader`] it builds from the movie, so a caller hands over bytes
/// and takes [`Sample`]s. It reaches for no source of its own: when to read and
/// from where stay with the caller.
///
/// # Contract
///
/// * The file is handed over as it arrives, cut anywhere, and the samples it
///   completed are taken from [`poll_sample`](Self::poll_sample). The caller
///   drains before handing over more: samples are held until they are taken.
/// * The boxes the layout is made of are read into values, and the accessors
///   report them once they have arrived: [`file_type`](Self::file_type) and
///   [`movie`](Self::movie). Every other box is passed over, its payload offered
///   to the samples as media data — which box those bytes came from is not
///   asked, so an `mdat` under any framing reads the same.
/// * A movie fragment needs the movie its defaults come from, so one arriving
///   before any `moov` is
///   [`MissingMandatoryBox`](crate::FileErrorKind::MissingMandatoryBox). A file
///   carrying a second `moov` is
///   [`DuplicateBox`](crate::FileErrorKind::DuplicateBox), and so is one
///   carrying a second `ftyp`.
/// * A file carrying no `ftyp` reads all the same, though §4.3 has one placed
///   as early as possible. The mirror of this holds a writer to it — see
///   [`FragmentedWriter`](crate::FragmentedWriter).
/// * A box read into a value is gathered whole before it is read, so what it
///   declares is bounded — see [`with_limits`](Self::with_limits). A box
///   declaring no total at all is passed over rather than gathered, whatever its
///   type.
/// * An `Err` leaves the reader failed for good,
///   [`AlreadyFinished`](crate::FileErrorKind::AlreadyFinished) aside: every
///   later call reports that same failure again. The samples completed before it
///   are still there to take.
/// * [`finish`](Self::finish) declares the file over, and reports what either
///   layer beneath makes of the end of it: a box left open, or a sample short of
///   the data it claimed. Samples are still taken after it, but anything handed
///   over then, or a second [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::FileErrorKind::AlreadyFinished).
///
/// # Examples
///
/// ```
/// use isobmff::{FragmentedReader, FragmentedWriter, Sample, TrackExtendsBox};
/// # use isobmff_test_support::{file_type, fragmented_movie};
/// // A file of one fragment carrying two samples of track 1
/// let mut writer = FragmentedWriter::new();
/// writer.handle_file_type(file_type()).unwrap();
/// writer
///     .handle_movie(fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, 0)))
///     .unwrap();
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
/// // The file the writer laid down is drained as it hands the bytes over
/// let mut file = Vec::new();
/// while let Some(written) = writer.poll_output() {
///     file.extend_from_slice(&written);
/// }
///
/// // The file is handed over as it arrives, in whatever lengths it comes
/// let mut reader = FragmentedReader::new();
/// for arriving in file.chunks(7) {
///     reader.handle_input(arriving).unwrap();
/// }
/// reader.finish().unwrap();
///
/// // The brands and the movie the file declared are there to read
/// assert_eq!(reader.file_type().unwrap().major_brand(), file_type().major_brand());
/// assert_eq!(reader.movie().unwrap().trak().len(), 1);
///
/// // The samples come back as they were laid out
/// let first = reader.poll_sample().unwrap();
/// assert_eq!((first.data(), first.decode_time()), (b"SAMP".as_slice(), 0));
///
/// let second = reader.poll_sample().unwrap();
/// assert_eq!(
///     (second.data(), second.decode_time()),
///     (b"DATA".as_slice(), 1_024)
/// );
/// assert_eq!(reader.poll_sample(), None);
/// ```
#[derive(Debug)]
pub struct FragmentedReader {
    boxes: BoxReader,
    samples: Option<SampleReader>,
    file_type: Option<FileTypeBox>,
    movie: Option<MovieBox>,
    gathering: Option<Gathering>,
    payload_limit: u64,
    sample_size_limit: u64,
    state: State,
}

impl FragmentedReader {
    /// Payload a box read into a value may declare, where the caller names no limit
    ///
    /// Sixteen mebibytes. A caller reading files whose `moov` reaches past that
    /// — a presentation of many tracks states a sample entry for each — names a
    /// limit of its own with [`with_limits`](Self::with_limits).
    pub const DEFAULT_PAYLOAD_LIMIT: u64 = 16 * 1024 * 1024;

    /// Creates a reader waiting at the start of a fragmented file
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
    /// one sample rather than the file. A box of the layout declaring more than
    /// `payload_limit` bytes of payload is
    /// [`PayloadLimitExceeded`](crate::FileErrorKind::PayloadLimitExceeded)
    /// before a byte of it is gathered; a sample declaring more than
    /// `sample_size_limit` bytes is what
    /// [`SampleReader::with_sample_size_limit`](SampleReader::with_sample_size_limit)
    /// makes of it.
    #[must_use]
    pub const fn with_limits(payload_limit: u64, sample_size_limit: u64) -> Self {
        Self {
            boxes: BoxReader::new(),
            samples: None,
            file_type: None,
            movie: None,
            gathering: None,
            payload_limit,
            sample_size_limit,
            state: State::Reading,
        }
    }

    /// Takes the file as it arrived, and reads the samples it completes
    ///
    /// The input is taken whole. What it completed is then taken from
    /// [`poll_sample`](Self::poll_sample).
    ///
    /// # Errors
    ///
    /// * [`MissingMandatoryBox`](crate::FileErrorKind::MissingMandatoryBox): a
    ///   `moof` arrived before any `moov`.
    /// * [`DuplicateBox`](crate::FileErrorKind::DuplicateBox): a second `ftyp`
    ///   or `moov` arrived.
    /// * [`PayloadLimitExceeded`](crate::FileErrorKind::PayloadLimitExceeded): a
    ///   box of the layout declares more payload than the reader gathers.
    /// * [`Sequence`](crate::FileErrorKind::Sequence): what the framing of the
    ///   file makes of the input.
    /// * [`Box`](crate::FileErrorKind::Box): a box of the layout does not
    ///   decode.
    /// * [`Sample`](crate::FileErrorKind::Sample): what the samples make of a
    ///   fragment or the media data beside it.
    /// * [`AlreadyFinished`](crate::FileErrorKind::AlreadyFinished): the file
    ///   was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn handle_input(&mut self, input: &[u8]) -> Result<(), FileError> {
        self.reading()?;

        if let Err(failure) = self.boxes.handle_input(input) {
            return Err(self.fail(failure.into()));
        }

        self.read_framed()
    }

    /// Takes the next sample the file handed over so far completed
    ///
    /// Reports `None` once they are used up: more of the file is needed. Failure
    /// is reported by the calls that take it, so this one never fails — a failed
    /// reader hands over the samples it had already completed, then `None` from
    /// there on.
    pub fn poll_sample(&mut self) -> Option<Sample> {
        self.samples.as_mut()?.poll_sample()
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
    /// * [`Sequence`](crate::FileErrorKind::Sequence): the file ended inside a
    ///   box.
    /// * [`Sample`](crate::FileErrorKind::Sample): a sample a fragment declared
    ///   is short of the data it claimed.
    /// * [`AlreadyFinished`](crate::FileErrorKind::AlreadyFinished): the file
    ///   was already declared over.
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<(), FileError> {
        self.reading()?;

        if let Err(failure) = self.boxes.finish() {
            return Err(self.fail(failure.into()));
        }
        self.read_framed()?;

        let ended = self
            .samples
            .as_mut()
            .map_or(Ok(()), |samples| samples.finish());
        if let Err(failure) = ended {
            return Err(self.fail(failure.into()));
        }
        self.state = State::Finished;

        Ok(())
    }

    /// Returns `Ok` while the reader still takes what arrives
    fn reading(&self) -> Result<(), FileError> {
        match self.state {
            State::Reading => Ok(()),
            State::Finished => Err(FileError::already_finished()),
            State::Failed(failure) => Err(failure),
        }
    }

    /// Reads every box the framing has finished framing so far
    fn read_framed(&mut self) -> Result<(), FileError> {
        while let Some(event) = self.boxes.poll_event() {
            // Why not unreachable: an event was taken, so the framing names the
            // bytes it was read from, and the fallback is a degenerate range in
            // place of a panic the lints forbid.
            let extent = self.boxes.event_extent().unwrap_or(0..0);
            let outcome = match event {
                BoxEvent::Header(header) => self.begin_box(header, extent),
                BoxEvent::Payload(payload) => self.take_payload(payload, extent),
                BoxEvent::End => self.close_box(extent),
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

    /// Begins gathering the box `header` introduces, where the layout is made of it
    fn begin_box(&mut self, header: BoxHeader, extent: Range<u64>) -> Result<(), FileError> {
        let Some((value_box, declared)) = ValueBox::of(header.box_type()).zip(header.payload_len())
        else {
            return Ok(());
        };

        if declared > self.payload_limit {
            return Err(FileError::payload_limit_exceeded(
                header.box_type(),
                declared,
                self.payload_limit,
            ));
        }

        self.gathering = Some(Gathering {
            value_box,
            began_at: extent.start,
            // Why not reserve the declared length: the file declares it and the
            // limit only bounds it, so reserving would take memory for bytes
            // that may never arrive.
            payload: Vec::new(),
        });

        Ok(())
    }

    /// Gathers `payload` into the box being read, or offers it to the samples
    fn take_payload(&mut self, mut payload: Vec<u8>, extent: Range<u64>) -> Result<(), FileError> {
        if let Some(gathering) = self.gathering.as_mut() {
            if gathering.payload.is_empty() {
                gathering.payload = payload;
            } else {
                gathering.payload.append(&mut payload);
            }

            return Ok(());
        }

        let Some(samples) = self.samples.as_mut() else {
            return Ok(());
        };

        samples
            .handle_media_data(&payload, extent)
            .map_err(FileError::from)
    }

    /// Reads the box that ended, where it is one the layout is made of
    fn close_box(&mut self, extent: Range<u64>) -> Result<(), FileError> {
        let Some(gathering) = self.gathering.take() else {
            return Ok(());
        };

        match gathering.value_box {
            ValueBox::FileType => {
                if self.file_type.is_some() {
                    return Err(FileError::duplicate_box(FileTypeBox::BOX_TYPE));
                }
                self.file_type = Some(decoded::<FileTypeBox>(&gathering.payload)?);
            }
            ValueBox::Movie => {
                if self.movie.is_some() {
                    return Err(FileError::duplicate_box(MovieBox::BOX_TYPE));
                }
                let movie = decoded::<MovieBox>(&gathering.payload)?;

                self.samples = Some(SampleReader::with_sample_size_limit(
                    &movie,
                    self.sample_size_limit,
                )?);
                self.movie = Some(movie);
            }
            ValueBox::MovieFragment => {
                let movie_fragment = decoded::<MovieFragmentBox>(&gathering.payload)?;
                let Some(samples) = self.samples.as_mut() else {
                    return Err(FileError::missing_mandatory_box(MovieBox::BOX_TYPE));
                };

                samples.handle_movie_fragment(movie_fragment, gathering.began_at..extent.end)?;
            }
        }

        Ok(())
    }

    /// Fails the reader for good, and hands the failure back to report
    fn fail(&mut self, failure: FileError) -> FileError {
        self.state = State::Failed(failure);

        failure
    }
}

impl Default for FragmentedReader {
    fn default() -> Self {
        Self::new()
    }
}

/// Reads `payload` into the box it forms, naming that box on a failure
fn decoded<Value: BoxDecode + BoxDefinition>(payload: &[u8]) -> Result<Value, FileError> {
    Value::decode_payload(payload)
        .map_err(|failure| FileError::from(failure.in_container(Value::BOX_TYPE)))
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxType, FourCC};
    use isobmff_test_support::{file_type, fragmented_movie, framed, movie_fragment, written};

    use super::{BoxDefinition, FileError, FileTypeBox, FragmentedReader, MovieBox, SampleReader};
    use crate::{FileErrorKind, TrackExtendsBox};

    /// Movie of one track continued in fragments, whose defaults a `trex` states
    fn movie() -> MovieBox {
        fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, 0))
    }

    #[test]
    fn a_fragment_arriving_before_any_movie_is_rejected() {
        let file = [written(&file_type()), written(&movie_fragment())].concat();
        let mut reader = FragmentedReader::new();

        assert_eq!(
            reader.handle_input(&file),
            Err(FileError::missing_mandatory_box(MovieBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_second_movie_is_rejected() {
        let file = [written(&movie()), written(&movie())].concat();
        let mut reader = FragmentedReader::new();

        assert_eq!(
            reader.handle_input(&file),
            Err(FileError::duplicate_box(MovieBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_second_declaration_of_the_brands_is_rejected() {
        let file = [written(&file_type()), written(&file_type())].concat();
        let mut reader = FragmentedReader::new();

        assert_eq!(
            reader.handle_input(&file),
            Err(FileError::duplicate_box(FileTypeBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_file_declaring_no_brands_is_read_all_the_same() {
        let file = [written(&movie()), written(&movie_fragment())].concat();
        let mut reader = FragmentedReader::new();

        reader.handle_input(&file).unwrap();
        reader.finish().unwrap();

        assert_eq!(reader.file_type(), None);
        assert_eq!(reader.movie(), Some(&movie()));
    }

    #[test]
    fn a_box_of_the_layout_declaring_a_payload_past_the_limit_is_rejected() {
        let mut reader = FragmentedReader::with_limits(4, SampleReader::DEFAULT_SAMPLE_SIZE_LIMIT);

        assert_eq!(
            reader
                .handle_input(&written(&file_type()))
                .map_err(FileError::kind),
            Err(FileErrorKind::PayloadLimitExceeded)
        );
    }

    #[test]
    fn a_box_the_layout_passes_over_is_not_bounded_by_the_limit() {
        let file = framed(BoxType::compact(*b"mdat"), &[0x11; 64]);
        let mut reader = FragmentedReader::with_limits(0, SampleReader::DEFAULT_SAMPLE_SIZE_LIMIT);

        reader.handle_input(&file).unwrap();

        assert_eq!(reader.finish(), Ok(()));
    }

    #[test]
    fn a_box_of_the_layout_declaring_no_total_is_passed_over() {
        let mut file = vec![0x00, 0x00, 0x00, 0x00];
        file.extend_from_slice(b"moovPAYLOAD");

        let mut reader = FragmentedReader::new();

        reader.handle_input(&file).unwrap();
        reader.finish().unwrap();

        assert_eq!(reader.movie(), None);
    }

    #[test]
    fn a_box_of_the_layout_whose_payload_does_not_decode_names_that_box() {
        let mut reader = FragmentedReader::new();
        let failure = reader.handle_input(&framed(BoxType::compact(*b"moov"), b"AAAA"));

        assert_eq!(
            failure.map_err(|reported| reported
                .box_error()
                .map(|box_error| box_error.containers().collect::<Vec<_>>())),
            Err(Some(vec![FourCC::new(*b"moov")]))
        );
    }

    #[test]
    fn a_failed_reader_reports_the_same_failure_for_every_call_after_it() {
        let mut reader = FragmentedReader::new();
        let failure = FileError::duplicate_box(FileTypeBox::BOX_TYPE);
        let file = [written(&file_type()), written(&file_type())].concat();

        assert_eq!(reader.handle_input(&file), Err(failure));
        assert_eq!(reader.handle_input(&written(&movie())), Err(failure));
        assert_eq!(reader.finish(), Err(failure));
    }

    #[test]
    fn input_handed_over_after_finishing_is_rejected() {
        let mut reader = FragmentedReader::new();

        reader.finish().unwrap();

        assert_eq!(
            reader.handle_input(&written(&file_type())),
            Err(FileError::already_finished())
        );
        assert_eq!(reader.finish(), Err(FileError::already_finished()));
    }
}
