//! [`FragmentedWriter`], a fragmented movie file laid down as the samples come

use alloc::vec;
use alloc::vec::Vec;

use isobmff_boxes::{FileTypeBox, MediaDataBox, MovieBox, MovieFragmentBox};
use isobmff_core::{BoxDefinition, BoxEncode, BoxHeader, BoxType};
use isobmff_sequence::{BoxEvent, BoxWriter, EventBytes};

use crate::{FileError, Sample, SampleWriter};

/// Where the writer stands between calls
#[derive(Clone, Copy, Debug)]
enum State {
    /// Waiting for the brands the file opens with
    Opening,
    /// Waiting for the movie the fragments continue
    Declaring,
    /// Laying out fragment after fragment
    Writing,
    /// Told the samples are over, and taking nothing more
    Finished,
    /// Failed, and reporting that same failure for every call after it
    Failed(FileError),
}

/// Lays a fragmented movie file down, taking the samples as they come
///
/// The mirror of [`FragmentedReader`](crate::FragmentedReader): it holds the
/// layout of ISO/IEC 14496-12 Annex A.8 — the brands, the movie, then one movie
/// fragment after another with the media data beside it — and lays it down
/// through the box layer, so a caller hands over boxes and samples and takes
/// bytes. A [`SampleWriter`] lays the samples of a fragment out; this writer
/// settles what a fragment is written as and where it lands in the file. It
/// reaches for no destination of its own: when to write and to where stay with
/// the caller.
///
/// # Contract
///
/// * The layout is held to as the boxes are handed over: the `ftyp` first, as
///   early as §4.3 asks, the `moov` before any fragment. One that never came is
///   [`MissingMandatoryBox`](crate::FileErrorKind::MissingMandatoryBox) and one
///   that came again is
///   [`DuplicateBox`](crate::FileErrorKind::DuplicateBox).
/// * A fragment is opened by [`begin_fragment`](Self::begin_fragment), carries
///   the samples handed over next, and is laid down by
///   [`finish_fragment`](Self::finish_fragment) as the `moof` and the `mdat`
///   the sample layer made of it. What the samples themselves must hold to is
///   [`SampleWriter`]'s contract, and this writer reports it as
///   [`Sample`](crate::FileErrorKind::Sample).
/// * The bytes are taken from [`poll_output`](Self::poll_output), one
///   [`EventBytes`] a call, owned by whoever takes them. The caller drains
///   before handing over more: bytes are held until they are taken, so writing
///   on without polling has the writer hold the whole file.
/// * An `Err` leaves the writer failed for good,
///   [`AlreadyFinished`](crate::FileErrorKind::AlreadyFinished) aside: every
///   later call reports that same failure again. The bytes made before it are
///   still there to take.
/// * [`finish`](Self::finish) declares the file over. Bytes are still taken
///   after it, but anything handed over then, or a second
///   [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::FileErrorKind::AlreadyFinished).
///
/// # Examples
///
/// ```
/// use isobmff::{FragmentedWriter, Sample, TrackExtendsBox};
/// # use isobmff_test_support::{file_type, fragmented_movie};
/// // A file opening with its brands and the movie its fragments continue
/// let mut writer = FragmentedWriter::new();
/// writer.handle_file_type(file_type()).unwrap();
/// writer
///     .handle_movie(fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, 0)))
///     .unwrap();
///
/// // One fragment of two samples of track 1, lasting 1024 units each
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
/// // The bytes are drained as the writer hands them over
/// let mut file = Vec::new();
/// while let Some(written) = writer.poll_output() {
///     file.extend_from_slice(&written);
/// }
///
/// // The file opens with the brands, and the media data holds the samples end to end
/// assert_eq!(&file[4..8], b"ftyp");
/// assert!(file.ends_with(b"SAMPDATA"));
/// ```
#[derive(Debug)]
pub struct FragmentedWriter {
    boxes: BoxWriter,
    samples: SampleWriter,
    state: State,
}

impl FragmentedWriter {
    /// Creates a writer waiting for the brands the file opens with
    #[must_use]
    pub const fn new() -> Self {
        Self {
            boxes: BoxWriter::new(),
            samples: SampleWriter::new(),
            state: State::Opening,
        }
    }

    /// Takes the brands the file declares itself readable as, and lays them down
    ///
    /// # Errors
    ///
    /// * [`DuplicateBox`](crate::FileErrorKind::DuplicateBox): the brands were
    ///   handed over already.
    /// * [`Box`](crate::FileErrorKind::Box): the box does not write.
    /// * [`AlreadyFinished`](crate::FileErrorKind::AlreadyFinished): the file
    ///   was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn handle_file_type(&mut self, file_type: FileTypeBox) -> Result<(), FileError> {
        match self.state {
            State::Opening => {}
            State::Declaring | State::Writing => {
                return Err(self.fail(FileError::box_handed_over_twice(FileTypeBox::BOX_TYPE)));
            }
            State::Finished => return Err(FileError::already_finished()),
            State::Failed(failure) => return Err(failure),
        }

        self.write_value(&file_type, FileTypeBox::BOX_TYPE)?;
        self.state = State::Declaring;

        Ok(())
    }

    /// Takes the movie the fragments continue, and lays it down
    ///
    /// # Errors
    ///
    /// * [`MissingMandatoryBox`](crate::FileErrorKind::MissingMandatoryBox): the
    ///   brands the file opens with were not handed over first.
    /// * [`DuplicateBox`](crate::FileErrorKind::DuplicateBox): the movie was
    ///   handed over already.
    /// * [`Box`](crate::FileErrorKind::Box): the box does not write.
    /// * [`AlreadyFinished`](crate::FileErrorKind::AlreadyFinished): the file
    ///   was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn handle_movie(&mut self, movie: MovieBox) -> Result<(), FileError> {
        match self.state {
            State::Declaring => {}
            State::Opening => {
                return Err(self.fail(FileError::box_not_handed_over(FileTypeBox::BOX_TYPE)));
            }
            State::Writing => {
                return Err(self.fail(FileError::box_handed_over_twice(MovieBox::BOX_TYPE)));
            }
            State::Finished => return Err(FileError::already_finished()),
            State::Failed(failure) => return Err(failure),
        }

        self.write_value(&movie, MovieBox::BOX_TYPE)?;
        self.state = State::Writing;

        Ok(())
    }

    /// Opens a fragment, which the samples handed over next are laid out in
    ///
    /// `sequence_number` is what its `mfhd` states, which §8.8.5 has increase
    /// over the fragments of a presentation.
    ///
    /// # Errors
    ///
    /// * [`MissingMandatoryBox`](crate::FileErrorKind::MissingMandatoryBox): the
    ///   brands or the movie were not handed over first.
    /// * [`Sample`](crate::FileErrorKind::Sample): what the sample layer makes
    ///   of the call.
    /// * [`AlreadyFinished`](crate::FileErrorKind::AlreadyFinished): the file
    ///   was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn begin_fragment(&mut self, sequence_number: u32) -> Result<(), FileError> {
        self.laying_out()?;

        if let Err(failure) = self.samples.begin_fragment(sequence_number) {
            return Err(self.fail(failure.into()));
        }

        Ok(())
    }

    /// Takes a sample, and places it in the fragment that is open
    ///
    /// # Errors
    ///
    /// * [`MissingMandatoryBox`](crate::FileErrorKind::MissingMandatoryBox): the
    ///   brands or the movie were not handed over first.
    /// * [`Sample`](crate::FileErrorKind::Sample): what the sample layer makes
    ///   of the sample.
    /// * [`AlreadyFinished`](crate::FileErrorKind::AlreadyFinished): the file
    ///   was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), FileError> {
        self.laying_out()?;

        if let Err(failure) = self.samples.handle_sample(sample) {
            return Err(self.fail(failure.into()));
        }

        Ok(())
    }

    /// Closes the fragment that is open, and lays it down
    ///
    /// The `moof` and the `mdat` the sample layer made of it are written here,
    /// the media data moving into the file rather than being copied into it.
    ///
    /// # Errors
    ///
    /// * [`MissingMandatoryBox`](crate::FileErrorKind::MissingMandatoryBox): the
    ///   brands or the movie were not handed over first.
    /// * [`Sample`](crate::FileErrorKind::Sample): what the sample layer makes
    ///   of the fragment.
    /// * [`Box`](crate::FileErrorKind::Box): the `moof` or the `mdat` does not
    ///   write.
    /// * [`AlreadyFinished`](crate::FileErrorKind::AlreadyFinished): the file
    ///   was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn finish_fragment(&mut self) -> Result<(), FileError> {
        self.laying_out()?;

        if let Err(failure) = self.samples.finish_fragment() {
            return Err(self.fail(failure.into()));
        }

        self.lay_down_fragments()
    }

    /// Hands over the bytes the file has been laid down as so far
    ///
    /// Reports `None` once they are used up: more samples are needed, or the
    /// file is over. Failure is reported by the calls that take the boxes and
    /// the samples, so this one never fails — a failed writer hands over the
    /// bytes it had already made, then nothing from there on.
    pub fn poll_output(&mut self) -> Option<EventBytes> {
        self.boxes.poll_output()
    }

    /// Declares the file over
    ///
    /// # Errors
    ///
    /// * [`MissingMandatoryBox`](crate::FileErrorKind::MissingMandatoryBox): the
    ///   brands or the movie the layout requires were never handed over, so the
    ///   file the writer laid down is not one.
    /// * [`Sample`](crate::FileErrorKind::Sample): a fragment was left open.
    /// * [`AlreadyFinished`](crate::FileErrorKind::AlreadyFinished): the file
    ///   was already declared over.
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<(), FileError> {
        self.laying_out()?;

        if let Err(failure) = self.samples.finish() {
            return Err(self.fail(failure.into()));
        }
        self.lay_down_fragments()?;

        if let Err(failure) = self.boxes.finish() {
            return Err(self.fail(failure.into()));
        }
        self.state = State::Finished;

        Ok(())
    }

    /// Returns `Ok` while the writer takes the samples of a fragment
    fn laying_out(&mut self) -> Result<(), FileError> {
        // Why not one failure for both: a fragment lands in a file that declared
        // its brands and the movie it continues, so the state names which of the
        // two the layout is still waiting for.
        match self.state {
            State::Writing => Ok(()),
            State::Opening => Err(self.fail(FileError::box_not_handed_over(FileTypeBox::BOX_TYPE))),
            State::Declaring => Err(self.fail(FileError::box_not_handed_over(MovieBox::BOX_TYPE))),
            State::Finished => Err(FileError::already_finished()),
            State::Failed(failure) => Err(failure),
        }
    }

    /// Lays down every fragment the sample layer has closed
    fn lay_down_fragments(&mut self) -> Result<(), FileError> {
        while let Some((movie_fragment, media_data)) = self.samples.poll_fragment() {
            self.write_value(&movie_fragment, MovieFragmentBox::BOX_TYPE)?;
            self.lay_down(MediaDataBox::BOX_TYPE, media_data.into_data())?;
        }

        Ok(())
    }

    /// Lays `value` down as the whole box it forms
    fn write_value(&mut self, value: &impl BoxEncode, box_type: BoxType) -> Result<(), FileError> {
        let payload_len = value.payload_len();
        let Ok(length) = usize::try_from(payload_len) else {
            return Err(self.fail(past_every_buffer(box_type, payload_len)));
        };
        let mut payload = vec![0; length];

        if let Err(failure) = value.encode_payload(&mut payload) {
            return Err(self.fail(FileError::from(failure.in_container(box_type))));
        }

        self.lay_down(box_type, payload)
    }

    /// Lays one box down through the framing of the file
    fn lay_down(&mut self, box_type: BoxType, payload: Vec<u8>) -> Result<(), FileError> {
        let payload_len = payload.len() as u64;
        let Some(header) = BoxHeader::with_payload_len(box_type, payload_len) else {
            return Err(self.fail(past_every_buffer(box_type, payload_len)));
        };

        self.lay_down_step(BoxEvent::Header(header))?;
        if !payload.is_empty() {
            self.lay_down_step(BoxEvent::Payload(payload))?;
        }
        self.lay_down_step(BoxEvent::End)
    }

    /// Hands one step of the framing over, failing the writer where it is refused
    fn lay_down_step(&mut self, step: BoxEvent) -> Result<(), FileError> {
        if let Err(failure) = self.boxes.handle_event(step) {
            return Err(self.fail(failure.into()));
        }

        Ok(())
    }

    /// Fails the writer for good, and hands the failure back to report
    fn fail(&mut self, failure: FileError) -> FileError {
        self.state = State::Failed(failure);

        failure
    }
}

impl Default for FragmentedWriter {
    fn default() -> Self {
        Self::new()
    }
}

/// Reports a box longer than any buffer on this target, as `isobmff-core` names it
fn past_every_buffer(box_type: BoxType, payload_len: u64) -> FileError {
    // Why not a failure of its own: a payload past `usize`, and a header that
    // cannot measure one, both exceed every buffer this target can hold, which
    // is the short buffer `isobmff_core::BoxEncode::encode` folds them into.
    FileError::from(
        isobmff_core::Error::truncated_buffer(payload_len, usize::MAX as u64)
            .in_container(box_type),
    )
}

#[cfg(test)]
mod tests {
    use isobmff_test_support::{file_type, fragmented_movie};

    use super::{BoxDefinition, FileError, FileTypeBox, FragmentedWriter, MovieBox};
    use crate::{Sample, TrackExtendsBox};

    /// Movie of one track continued in fragments, whose defaults a `trex` states
    fn movie() -> MovieBox {
        fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, 0))
    }

    /// A sample of the track the movie declares
    fn sample() -> Sample {
        Sample::new(1, 0, 1_024, None, 0, 1, b"SAMP".to_vec())
    }

    #[test]
    fn a_movie_handed_over_before_the_brands_is_rejected() {
        let mut writer = FragmentedWriter::new();

        assert_eq!(
            writer.handle_movie(movie()),
            Err(FileError::box_not_handed_over(FileTypeBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_second_declaration_of_the_brands_is_rejected() {
        let mut writer = FragmentedWriter::new();

        writer.handle_file_type(file_type()).unwrap();

        assert_eq!(
            writer.handle_file_type(file_type()),
            Err(FileError::box_handed_over_twice(FileTypeBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_second_movie_is_rejected() {
        let mut writer = FragmentedWriter::new();

        writer.handle_file_type(file_type()).unwrap();
        writer.handle_movie(movie()).unwrap();

        assert_eq!(
            writer.handle_movie(movie()),
            Err(FileError::box_handed_over_twice(MovieBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_fragment_opened_before_the_movie_is_rejected() {
        let mut writer = FragmentedWriter::new();

        writer.handle_file_type(file_type()).unwrap();

        assert_eq!(
            writer.begin_fragment(1),
            Err(FileError::box_not_handed_over(MovieBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_sample_handed_over_before_the_brands_is_rejected() {
        let mut writer = FragmentedWriter::new();

        assert_eq!(
            writer.handle_sample(sample()),
            Err(FileError::box_not_handed_over(FileTypeBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_file_declared_over_without_the_boxes_its_layout_requires_is_rejected() {
        let mut writer = FragmentedWriter::new();

        assert_eq!(
            writer.finish(),
            Err(FileError::box_not_handed_over(FileTypeBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_failed_writer_reports_the_same_failure_for_every_call_after_it() {
        let mut writer = FragmentedWriter::new();
        let failure = FileError::box_not_handed_over(FileTypeBox::BOX_TYPE);

        assert_eq!(writer.handle_movie(movie()), Err(failure));
        assert_eq!(writer.handle_file_type(file_type()), Err(failure));
        assert_eq!(writer.finish(), Err(failure));
    }

    #[test]
    fn a_failed_writer_hands_over_the_bytes_it_had_already_laid_down() {
        let mut writer = FragmentedWriter::new();

        writer.handle_file_type(file_type()).unwrap();

        assert!(writer.handle_file_type(file_type()).is_err());

        let written = writer.poll_output().unwrap();

        assert_eq!(*written, *b"\0\0\0\x18ftyp");
    }

    #[test]
    fn anything_handed_over_after_finishing_is_rejected() {
        let mut writer = FragmentedWriter::new();

        writer.handle_file_type(file_type()).unwrap();
        writer.handle_movie(movie()).unwrap();
        writer.finish().unwrap();

        assert_eq!(
            writer.handle_sample(sample()),
            Err(FileError::already_finished())
        );
        assert_eq!(writer.finish(), Err(FileError::already_finished()));
    }
}
