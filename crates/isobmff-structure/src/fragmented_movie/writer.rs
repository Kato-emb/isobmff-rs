//! [`FragmentedWriter`], a fragmented movie file laid down as the samples come

use alloc::vec::Vec;

use isobmff_boxes::{FileTypeBox, MediaDataBox, MovieBox};
use isobmff_core::{BoxDefinition, BoxEncode, BoxType, FourCC};
use isobmff_sample::{MovieFragmentWriter, Sample};
use isobmff_sequence::{BoxEvent, BoxWriter, EventBytes};

use super::{FragmentedDisposition, FragmentedStructure};
use crate::{Error, whole_box_header, whole_payload};

/// Lays a fragmented movie file down, taking the samples as they come
///
/// The mirror of [`FragmentedReader`](crate::FragmentedReader):
/// it wires the layers that write a fragmented movie file of ISO/IEC 14496-12
/// Annex A.8 — the structure that holds the order of the top-level boxes, the
/// writing of each box whole, the laying out of the samples of a fragment as
/// its `moof` and the media data beside it, and the framing of the file — so a
/// caller hands over boxes and samples and takes bytes. It holds no rule of
/// its own, and reaches for no destination: when to write and to where stay
/// with the caller.
///
/// # Contract
///
/// * The order of the boxes is the structure's, held to as they are handed
///   over: the `ftyp` first, the `moov` once and before any fragment. A
///   box handed over out of that order is
///   [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder) or
///   [`DuplicateBox`](crate::ErrorKind::DuplicateBox), and a file
///   declared over without a `moov` is
///   [`MissingMandatoryBox`](crate::ErrorKind::MissingMandatoryBox).
/// * The `ftyp` handed over is laid down as it stands. Where none was handed
///   over, the writer lays its own down before the `moov`: `iso6` as its
///   `major_brand` and its one `compatible_brands` entry, with
///   `minor_version` 0, the brand the widest layout it lays down requires
///   (§8.8.7.1, Annex E.9).
/// * A fragment is opened by [`begin_fragment`](Self::begin_fragment) or
///   [`begin_fragment_continuing`](Self::begin_fragment_continuing),
///   carries the samples handed over next, and is laid down by
///   [`finish_fragment`](Self::finish_fragment) as the `moof` and the `mdat`
///   the sample layer made of it. What the samples themselves must hold to
///   is [`MovieFragmentWriter`]'s contract, reported as
///   [`Sample`](crate::ErrorKind::Sample).
/// * The bytes are taken from [`poll_output`](Self::poll_output), one
///   [`EventBytes`] a call, owned by whoever takes them. The caller drains
///   before handing over more: bytes are held until they are taken, so
///   writing on without polling has the writer hold the whole file.
/// * An `Err` leaves the writer failed for good,
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished) aside:
///   every later call reports that same failure again. The bytes made before
///   it are still there to take.
/// * [`finish`](Self::finish) declares the file over. Bytes are still taken
///   after it, but anything handed over then, or a second
///   [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished).
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{SampleFlags, TrackExtendsBox};
/// use isobmff_sample::Sample;
/// use isobmff_structure::FragmentedWriter;
/// # use isobmff_test_support::fragmented_movie;
/// // A file handed no brands, only the movie its fragments continue
/// let mut writer = FragmentedWriter::new();
/// writer.handle_movie(fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO)))?;
///
/// // One fragment of two samples of track 1, lasting 1024 units each
/// writer.begin_fragment(1)?;
/// writer.handle_sample(Sample::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec()))?;
/// writer.handle_sample(Sample::new(1, 1_024, 1_024, 0, SampleFlags::ZERO, 1, b"DATA".to_vec()))?;
/// writer.finish_fragment()?;
/// writer.finish()?;
///
/// // The bytes are drained as the writer hands them over
/// let mut file = Vec::new();
/// while let Some(written) = writer.poll_output() {
///     file.extend_from_slice(&written);
/// }
///
/// // The file opens with the brands the writer declares, and the media data holds the samples end to end
/// assert_eq!(&file[4..8], b"ftyp");
/// assert!(file.ends_with(b"SAMPDATA"));
/// # Ok::<(), isobmff_structure::Error>(())
/// ```
#[derive(Debug)]
pub struct FragmentedWriter {
    boxes: BoxWriter,
    structure: FragmentedStructure,
    samples: MovieFragmentWriter,
    state: State,
}

/// Where the writer stands between calls
#[derive(Clone, Copy, Debug)]
enum State {
    /// Laying the file down as the boxes and the samples come
    Writing,
    /// Told the samples are over, and taking nothing more
    Finished,
    /// Failed, and reporting that same failure for every call after it
    Failed(Error),
}

impl FragmentedWriter {
    /// Creates a writer waiting at the start of a fragmented movie file
    #[must_use]
    pub const fn new() -> Self {
        Self {
            boxes: BoxWriter::new(),
            structure: FragmentedStructure::new(),
            samples: MovieFragmentWriter::new(),
            state: State::Writing,
        }
    }

    /// Takes the brands the file declares itself readable as, and lays them down
    ///
    /// # Errors
    ///
    /// * [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder): a box
    ///   was handed over before them.
    /// * [`Box`](crate::ErrorKind::Box): the box does not write.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn handle_file_type(&mut self, file_type: FileTypeBox) -> Result<(), Error> {
        self.writing()?;
        self.write_value(&file_type)
    }

    /// Takes the movie the fragments continue, and lays it down
    ///
    /// # Errors
    ///
    /// * [`DuplicateBox`](crate::ErrorKind::DuplicateBox): the movie
    ///   was handed over already.
    /// * [`Box`](crate::ErrorKind::Box): the box does not write.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn handle_movie(&mut self, movie: MovieBox) -> Result<(), Error> {
        self.writing()?;
        if self.structure.is_at_start() {
            self.write_value(&default_file_type())?;
        }
        self.write_value(&movie)
    }

    /// Opens a fragment, which the samples handed over next are laid out in
    ///
    /// `sequence_number` is what its `mfhd` states, which §8.8.5 has increase
    /// over the fragments of a presentation.
    ///
    /// # Errors
    ///
    /// * [`Sample`](crate::ErrorKind::Sample): what the sample layer
    ///   makes of the call.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn begin_fragment(&mut self, sequence_number: u32) -> Result<(), Error> {
        self.writing()?;
        self.samples
            .begin_fragment(sequence_number)
            .map_err(|failure| self.fail(failure.into()))
    }

    /// Opens a fragment in which every track continues where the samples written for it reach, as [`MovieFragmentWriter::begin_fragment_continuing`] places them
    ///
    /// `sequence_number` is what its `mfhd` states, as for
    /// [`begin_fragment`](Self::begin_fragment).
    ///
    /// # Errors
    ///
    /// * [`Sample`](crate::ErrorKind::Sample): what the sample layer
    ///   makes of the call.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn begin_fragment_continuing(&mut self, sequence_number: u32) -> Result<(), Error> {
        self.writing()?;
        self.samples
            .begin_fragment_continuing(sequence_number)
            .map_err(|failure| self.fail(failure.into()))
    }

    /// Takes a sample, and places it in the fragment that is open
    ///
    /// # Errors
    ///
    /// * [`Sample`](crate::ErrorKind::Sample): what the sample layer
    ///   makes of the sample.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), Error> {
        self.writing()?;
        self.samples
            .handle_sample(sample)
            .map_err(|failure| self.fail(failure.into()))
    }

    /// Closes the fragment that is open, and lays it down
    ///
    /// The `moof` and the `mdat` the sample layer made of it are written here,
    /// the media data moving into the file rather than being copied into it.
    ///
    /// # Errors
    ///
    /// * [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder): the
    ///   movie was not handed over first.
    /// * [`Sample`](crate::ErrorKind::Sample): what the sample layer
    ///   makes of the fragment.
    /// * [`Box`](crate::ErrorKind::Box): the `moof` or the `mdat`
    ///   does not write.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn finish_fragment(&mut self) -> Result<(), Error> {
        self.writing()?;
        let (movie_fragment, media_data) = self
            .samples
            .finish_fragment()
            .map_err(|failure| self.fail(failure.into()))?;

        self.write_value(&movie_fragment)?;
        self.lay_down(MediaDataBox::BOX_TYPE, media_data)
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
    /// * [`Sample`](crate::ErrorKind::Sample): a fragment was left
    ///   open.
    /// * [`MissingMandatoryBox`](crate::ErrorKind::MissingMandatoryBox):
    ///   the movie was never handed over, so the file laid down is not a
    ///   fragmented movie file.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was already declared over.
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.writing()?;
        self.samples
            .finish()
            .map_err(|failure| self.fail(failure.into()))?;
        self.structure
            .finish()
            .map_err(|failure| self.fail(failure))?;
        self.boxes
            .finish()
            .map_err(|failure| self.fail(failure.into()))?;
        self.state = State::Finished;

        Ok(())
    }

    /// Returns `Ok` while the writer still takes boxes and samples
    const fn writing(&self) -> Result<(), Error> {
        match self.state {
            State::Writing => Ok(()),
            State::Finished => Err(Error::already_finished()),
            State::Failed(failure) => Err(failure),
        }
    }

    /// Lays `value` down as the whole box it forms
    fn write_value<Value: BoxEncode + BoxDefinition>(
        &mut self,
        value: &Value,
    ) -> Result<(), Error> {
        let payload = whole_payload(value).map_err(|failure| self.fail(failure))?;

        self.lay_down(Value::BOX_TYPE, payload)
    }

    /// Lays one box down where the structure places it, through the framing of the file
    ///
    /// A box the structure passes over is refused as
    /// [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder).
    fn lay_down(&mut self, box_type: BoxType, payload: Vec<u8>) -> Result<(), Error> {
        let header = whole_box_header(box_type, payload.len() as u64)
            .map_err(|failure| self.fail(failure))?;

        match self
            .structure
            .handle_box_type(box_type)
            .map_err(|failure| self.fail(failure))?
        {
            FragmentedDisposition::FileType
            | FragmentedDisposition::Movie
            | FragmentedDisposition::MovieFragment
            | FragmentedDisposition::MediaData => {}
            FragmentedDisposition::SegmentIndex
            | FragmentedDisposition::MovieFragmentRandomAccess
            | FragmentedDisposition::Skip => {
                return Err(self.fail(Error::box_out_of_order(box_type)));
            }
        }

        self.lay_down_step(BoxEvent::Header(header))?;
        if !payload.is_empty() {
            self.lay_down_step(BoxEvent::Payload(payload))?;
        }
        self.lay_down_step(BoxEvent::End)
    }

    /// Hands one step of the framing over, failing the writer where it is refused
    fn lay_down_step(&mut self, step: BoxEvent) -> Result<(), Error> {
        self.boxes
            .handle_event(step)
            .map_err(|failure| self.fail(failure.into()))
    }

    /// Fails the writer for good, and hands the failure back to report
    const fn fail(&mut self, failure: Error) -> Error {
        self.state = State::Failed(failure);

        failure
    }
}

impl Default for FragmentedWriter {
    fn default() -> Self {
        Self::new()
    }
}

/// Brands the writer declares where none were handed over, those the widest layout it lays down requires
fn default_file_type() -> FileTypeBox {
    FileTypeBox::new(FourCC::new(*b"iso6"), 0, alloc::vec![FourCC::new(*b"iso6")])
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use isobmff_boxes::{FileTypeBox, MovieBox, MovieFragmentBox, SampleFlags, TrackExtendsBox};
    use isobmff_core::{BoxDecode, BoxDefinition};
    use isobmff_sample::Sample;
    use isobmff_test_support::{file_type, fragmented_movie};

    use super::{Error, FragmentedWriter, default_file_type};
    use crate::ErrorKind;

    /// Movie of one track continued in fragments, whose defaults a `trex` states
    fn movie() -> MovieBox {
        fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO))
    }

    /// A sample of the track the movie declares
    fn sample() -> Sample {
        Sample::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec())
    }

    /// The bytes the writer has laid down, drained to the end
    fn drained(writer: &mut FragmentedWriter) -> Vec<u8> {
        let mut file = Vec::new();
        while let Some(written) = writer.poll_output() {
            file.extend_from_slice(&written);
        }

        file
    }

    #[test]
    fn a_file_handed_no_brands_opens_with_the_brands_the_writer_declares() {
        let mut writer = FragmentedWriter::new();

        writer.handle_movie(movie()).unwrap();
        writer.finish().unwrap();
        let file = drained(&mut writer);

        assert_eq!(
            FileTypeBox::decode(&file).map(|(file_type, rest)| (file_type, rest.get(4..8))),
            Ok((default_file_type(), Some(b"moov".as_slice())))
        );
    }

    #[test]
    fn the_brands_handed_over_are_laid_down_as_they_stand() {
        let mut writer = FragmentedWriter::new();

        writer.handle_file_type(file_type()).unwrap();
        writer.handle_movie(movie()).unwrap();
        writer.finish().unwrap();
        let file = drained(&mut writer);

        assert_eq!(
            FileTypeBox::decode(&file).map(|(file_type, rest)| (file_type, rest.get(4..8))),
            Ok((file_type(), Some(b"moov".as_slice())))
        );
    }

    #[test]
    fn a_fragment_closed_before_the_movie_is_rejected() {
        let mut writer = FragmentedWriter::new();

        writer.handle_file_type(file_type()).unwrap();
        writer.begin_fragment(1).unwrap();

        assert_eq!(
            writer.finish_fragment(),
            Err(Error::box_out_of_order(MovieFragmentBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_sample_handed_over_while_no_fragment_is_open_is_rejected() {
        let mut writer = FragmentedWriter::new();

        assert_eq!(
            writer.handle_sample(sample()).map_err(Error::kind),
            Err(ErrorKind::Sample(isobmff_sample::ErrorKind::NoFragmentOpen))
        );
    }

    #[test]
    fn a_file_declared_over_without_a_movie_is_rejected() {
        let mut writer = FragmentedWriter::new();

        writer.handle_file_type(file_type()).unwrap();

        assert_eq!(
            writer.finish(),
            Err(Error::missing_mandatory_box(MovieBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_failed_writer_reports_the_same_failure_for_every_call_after_it() {
        let mut writer = FragmentedWriter::new();
        let failure = Error::box_out_of_order(FileTypeBox::BOX_TYPE);

        writer.handle_movie(movie()).unwrap();

        assert_eq!(writer.handle_file_type(file_type()), Err(failure));
        assert_eq!(writer.begin_fragment(1), Err(failure));
        assert_eq!(writer.finish(), Err(failure));
    }

    #[test]
    fn a_failed_writer_hands_over_the_bytes_it_had_already_laid_down() {
        let mut writer = FragmentedWriter::new();

        writer.handle_file_type(file_type()).unwrap();

        assert!(writer.handle_file_type(file_type()).is_err());

        assert_eq!(*writer.poll_output().unwrap(), *b"\0\0\0\x18ftyp");
    }

    #[test]
    fn anything_handed_over_after_finishing_is_rejected() {
        let mut writer = FragmentedWriter::new();

        writer.handle_file_type(file_type()).unwrap();
        writer.handle_movie(movie()).unwrap();
        writer.finish().unwrap();

        assert_eq!(
            writer.handle_sample(sample()),
            Err(Error::already_finished())
        );
        assert_eq!(writer.finish(), Err(Error::already_finished()));
    }
}
