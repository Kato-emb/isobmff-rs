//! [`MediaSegmentWriter`], a media segment laid down as the samples come

use alloc::vec::Vec;

use isobmff_boxes::{MediaDataBox, SegmentTypeBox};
use isobmff_core::{BoxDefinition, BoxEncode, BoxType};
use isobmff_sample::{MovieFragmentWriter, Sample};
use isobmff_sequence::{BoxEvent, BoxWriter, EventBytes};

use super::{MediaSegmentDisposition, MediaSegmentStructure};
use crate::{StructureError, whole_box_header, whole_payload};

/// Lays a media segment down, taking the samples as they come
///
/// The mirror of [`MediaSegmentReader`](crate::MediaSegmentReader): it wires
/// the layers that write a media segment of ISO/IEC 14496-12 §8.16 — the
/// structure that holds the order of the top-level boxes, the writing of
/// each box whole, the laying out of the samples of a fragment as its `moof`
/// and the media data beside it, and the framing of the segment — so a
/// caller hands over brands and samples and takes bytes. The movie the
/// segment continues is not written: a segment carries none. It holds no
/// rule of its own, and reaches for no destination: when to write and to
/// where stay with the caller.
///
/// # Contract
///
/// * The order of the boxes is the structure's, held to as they are handed
///   over: the `styp` first if at all, then the fragments. A `styp` handed
///   over after another box is
///   [`BoxOutOfOrder`](crate::StructureErrorKind::BoxOutOfOrder), and a
///   segment declared over without a fragment is
///   [`MissingMandatoryBox`](crate::StructureErrorKind::MissingMandatoryBox).
/// * A fragment is opened by [`begin_fragment`](Self::begin_fragment),
///   carries the samples handed over next, and is laid down by
///   [`finish_fragment`](Self::finish_fragment) as the `moof` and the `mdat`
///   the sample layer made of it. What the samples themselves must hold to
///   is [`MovieFragmentWriter`]'s contract, reported as
///   [`Sample`](crate::StructureErrorKind::Sample); a segment written apart
///   from the ones before it starts the decode time of each track where its
///   first sample states, since every fragment states one.
/// * The bytes are taken from [`poll_output`](Self::poll_output), one
///   [`EventBytes`] a call, owned by whoever takes them. The caller drains
///   before handing over more: bytes are held until they are taken, so
///   writing on without polling has the writer hold the whole segment.
/// * An `Err` leaves the writer failed for good,
///   [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished) aside:
///   every later call reports that same failure again. The bytes made before
///   it are still there to take.
/// * [`finish`](Self::finish) declares the segment over. Bytes are still
///   taken after it, but anything handed over then, or a second
///   [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished).
///
/// # Examples
///
/// ```
/// use isobmff_sample::Sample;
/// use isobmff_structure::MediaSegmentWriter;
/// # use isobmff_test_support::segment_type;
/// // A segment opening with its brands
/// let mut writer = MediaSegmentWriter::new();
/// writer.handle_segment_type(segment_type())?;
///
/// // One fragment of two samples of track 1, lasting 1024 units each
/// writer.begin_fragment(1)?;
/// writer.handle_sample(Sample::new(1, 0, 1_024, 0, 0, 1, b"SAMP".to_vec()))?;
/// writer.handle_sample(Sample::new(1, 1_024, 1_024, 0, 0, 1, b"DATA".to_vec()))?;
/// writer.finish_fragment()?;
/// writer.finish()?;
///
/// // The bytes are drained as the writer hands them over
/// let mut segment = Vec::new();
/// while let Some(written) = writer.poll_output() {
///     segment.extend_from_slice(&written);
/// }
///
/// // The segment opens with the brands, and the media data holds the samples end to end
/// assert_eq!(&segment[4..8], b"styp");
/// assert!(segment.ends_with(b"SAMPDATA"));
/// # Ok::<(), isobmff_structure::StructureError>(())
/// ```
#[derive(Debug)]
pub struct MediaSegmentWriter {
    boxes: BoxWriter,
    structure: MediaSegmentStructure,
    samples: MovieFragmentWriter,
    state: State,
}

/// Where the writer stands between calls
#[derive(Clone, Copy, Debug)]
enum State {
    /// Laying the segment down as the brands and the samples come
    Writing,
    /// Told the samples are over, and taking nothing more
    Finished,
    /// Failed, and reporting that same failure for every call after it
    Failed(StructureError),
}

impl MediaSegmentWriter {
    /// Creates a writer waiting at the start of a media segment
    #[must_use]
    pub const fn new() -> Self {
        Self {
            boxes: BoxWriter::new(),
            structure: MediaSegmentStructure::new(),
            samples: MovieFragmentWriter::new(),
            state: State::Writing,
        }
    }

    /// Takes the brands the segment declares itself readable as, and lays them down
    ///
    /// # Errors
    ///
    /// * [`BoxOutOfOrder`](crate::StructureErrorKind::BoxOutOfOrder): a box
    ///   was laid down before them.
    /// * [`Box`](crate::StructureErrorKind::Box): the box does not write.
    /// * [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished): the
    ///   segment was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn handle_segment_type(
        &mut self,
        segment_type: SegmentTypeBox,
    ) -> Result<(), StructureError> {
        self.writing()?;
        self.write_value(&segment_type)
    }

    /// Opens a fragment, which the samples handed over next are laid out in
    ///
    /// `sequence_number` is what its `mfhd` states, which §8.8.5 has increase
    /// over the fragments of a presentation.
    ///
    /// # Errors
    ///
    /// * [`Sample`](crate::StructureErrorKind::Sample): what the sample layer
    ///   makes of the call.
    /// * [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished): the
    ///   segment was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn begin_fragment(&mut self, sequence_number: u32) -> Result<(), StructureError> {
        self.writing()?;
        self.samples
            .begin_fragment(sequence_number)
            .map_err(|failure| self.fail(failure.into()))
    }

    /// Takes a sample, and places it in the fragment that is open
    ///
    /// # Errors
    ///
    /// * [`Sample`](crate::StructureErrorKind::Sample): what the sample layer
    ///   makes of the sample.
    /// * [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished): the
    ///   segment was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), StructureError> {
        self.writing()?;
        self.samples
            .handle_sample(sample)
            .map_err(|failure| self.fail(failure.into()))
    }

    /// Closes the fragment that is open, and lays it down
    ///
    /// The `moof` and the `mdat` the sample layer made of it are written here,
    /// the media data moving into the segment rather than being copied into
    /// it.
    ///
    /// # Errors
    ///
    /// * [`Sample`](crate::StructureErrorKind::Sample): what the sample layer
    ///   makes of the fragment.
    /// * [`Box`](crate::StructureErrorKind::Box): the `moof` or the `mdat`
    ///   does not write.
    /// * [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished): the
    ///   segment was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn finish_fragment(&mut self) -> Result<(), StructureError> {
        self.writing()?;
        let (movie_fragment, media_data) = self
            .samples
            .finish_fragment()
            .map_err(|failure| self.fail(failure.into()))?;

        self.write_value(&movie_fragment)?;
        self.lay_down(MediaDataBox::BOX_TYPE, media_data)
    }

    /// Hands over the bytes the segment has been laid down as so far
    ///
    /// Reports `None` once they are used up: more samples are needed, or the
    /// segment is over. Failure is reported by the calls that take the brands
    /// and the samples, so this one never fails — a failed writer hands over
    /// the bytes it had already made, then nothing from there on.
    pub fn poll_output(&mut self) -> Option<EventBytes> {
        self.boxes.poll_output()
    }

    /// Declares the segment over
    ///
    /// # Errors
    ///
    /// * [`Sample`](crate::StructureErrorKind::Sample): a fragment was left
    ///   open.
    /// * [`MissingMandatoryBox`](crate::StructureErrorKind::MissingMandatoryBox):
    ///   no fragment was laid down, so what was laid down is not a media
    ///   segment.
    /// * [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished): the
    ///   segment was already declared over.
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<(), StructureError> {
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

    /// Returns `Ok` while the writer still takes brands and samples
    const fn writing(&self) -> Result<(), StructureError> {
        match self.state {
            State::Writing => Ok(()),
            State::Finished => Err(StructureError::already_finished()),
            State::Failed(failure) => Err(failure),
        }
    }

    /// Lays `value` down as the whole box it forms
    fn write_value<Value: BoxEncode + BoxDefinition>(
        &mut self,
        value: &Value,
    ) -> Result<(), StructureError> {
        let payload = whole_payload(value).map_err(|failure| self.fail(failure))?;

        self.lay_down(Value::BOX_TYPE, payload)
    }

    /// Lays one box down where the structure places it, through the framing of the segment
    ///
    /// A box the structure passes over is refused as
    /// [`BoxOutOfOrder`](crate::StructureErrorKind::BoxOutOfOrder).
    fn lay_down(&mut self, box_type: BoxType, payload: Vec<u8>) -> Result<(), StructureError> {
        let header = whole_box_header(box_type, payload.len() as u64)
            .map_err(|failure| self.fail(failure))?;

        match self
            .structure
            .handle_box_type(box_type)
            .map_err(|failure| self.fail(failure))?
        {
            MediaSegmentDisposition::SegmentType
            | MediaSegmentDisposition::MovieFragment
            | MediaSegmentDisposition::MediaData => {}
            MediaSegmentDisposition::Skip => {
                return Err(self.fail(StructureError::box_out_of_order(box_type)));
            }
        }

        self.lay_down_step(BoxEvent::Header(header))?;
        if !payload.is_empty() {
            self.lay_down_step(BoxEvent::Payload(payload))?;
        }
        self.lay_down_step(BoxEvent::End)
    }

    /// Hands one step of the framing over, failing the writer where it is refused
    fn lay_down_step(&mut self, step: BoxEvent) -> Result<(), StructureError> {
        self.boxes
            .handle_event(step)
            .map_err(|failure| self.fail(failure.into()))
    }

    /// Fails the writer for good, and hands the failure back to report
    const fn fail(&mut self, failure: StructureError) -> StructureError {
        self.state = State::Failed(failure);

        failure
    }
}

impl Default for MediaSegmentWriter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use isobmff_boxes::{MovieFragmentBox, SegmentTypeBox};
    use isobmff_core::BoxDefinition;
    use isobmff_sample::SampleErrorKind;
    use isobmff_test_support::segment_type;

    use super::super::tests::sample;
    use super::{MediaSegmentWriter, StructureError};
    use crate::StructureErrorKind;

    #[test]
    fn a_segment_declaring_no_brands_is_laid_down_all_the_same() {
        let mut writer = MediaSegmentWriter::new();

        writer.begin_fragment(1).unwrap();
        writer.finish_fragment().unwrap();

        assert_eq!(writer.finish(), Ok(()));
        assert!(writer.poll_output().unwrap().ends_with(b"moof"));
    }

    #[test]
    fn brands_handed_over_after_a_fragment_are_rejected() {
        let mut writer = MediaSegmentWriter::new();

        writer.begin_fragment(1).unwrap();
        writer.finish_fragment().unwrap();

        assert_eq!(
            writer.handle_segment_type(segment_type()),
            Err(StructureError::box_out_of_order(SegmentTypeBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_sample_handed_over_while_no_fragment_is_open_is_rejected() {
        let mut writer = MediaSegmentWriter::new();

        assert_eq!(
            writer.handle_sample(sample()).map_err(StructureError::kind),
            Err(StructureErrorKind::Sample(SampleErrorKind::NoFragmentOpen))
        );
    }

    #[test]
    fn a_segment_declared_over_without_a_fragment_is_rejected() {
        let mut writer = MediaSegmentWriter::new();

        writer.handle_segment_type(segment_type()).unwrap();

        assert_eq!(
            writer.finish(),
            Err(StructureError::missing_mandatory_box(
                MovieFragmentBox::BOX_TYPE
            ))
        );
    }

    #[test]
    fn a_failed_writer_reports_the_same_failure_for_every_call_after_it() {
        let mut writer = MediaSegmentWriter::new();

        writer.handle_segment_type(segment_type()).unwrap();
        let failure = writer.finish().unwrap_err();

        assert_eq!(writer.handle_segment_type(segment_type()), Err(failure));
        assert_eq!(writer.begin_fragment(1), Err(failure));
        assert_eq!(writer.finish(), Err(failure));
    }

    #[test]
    fn a_failed_writer_hands_over_the_bytes_it_had_already_laid_down() {
        let mut writer = MediaSegmentWriter::new();

        writer.handle_segment_type(segment_type()).unwrap();

        assert!(writer.finish().is_err());

        assert_eq!(*writer.poll_output().unwrap(), *b"\0\0\0\x18styp");
    }

    #[test]
    fn anything_handed_over_after_finishing_is_rejected() {
        let mut writer = MediaSegmentWriter::new();

        writer.begin_fragment(1).unwrap();
        writer.finish_fragment().unwrap();
        writer.finish().unwrap();

        assert_eq!(
            writer.handle_sample(sample()),
            Err(StructureError::already_finished())
        );
        assert_eq!(writer.finish(), Err(StructureError::already_finished()));
    }
}
