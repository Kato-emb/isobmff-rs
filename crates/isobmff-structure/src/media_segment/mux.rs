//! [`MediaSegmentMuxFsm`], a media segment laid down as the samples come

use alloc::vec::Vec;

use isobmff_boxes::{MediaDataBox, MovieBox, SegmentTypeBox};
use isobmff_core::{BoxDefinition, BoxEncode, BoxType};
use isobmff_sample::{MovieFragmentWriter, Sample};
use isobmff_sequence::EventBytes;

use super::{MediaSegmentDisposition, MediaSegmentStructure};
use crate::mux_output::MuxOutput;
use crate::{Error, whole_box_header, whole_payload};

/// Lays a media segment down, taking the samples as they come
///
/// The mirror of [`MediaSegmentDemuxFsm`](crate::MediaSegmentDemuxFsm): it wires
/// the layers that write a media segment of ISO/IEC 14496-12 §8.16 — the
/// structure that holds the order of the top-level boxes, the writing of
/// each box whole, the laying out of the samples of a fragment as its `moof`
/// and the media data beside it, and the framing of the segment — so a
/// caller hands over brands and samples and takes bytes. The movie the
/// segment continues — that of the initialization segment — is taken when
/// the mux FSM is made, and not written: a segment carries none. Beside the
/// structure it holds two rules of its own: a `styp` comes first if at all,
/// and none listing a brand that forbids the `default-base-is-moof` it writes
/// is laid down. It reaches for no destination: when to write and to where
/// stay with the caller.
///
/// # Contract
///
/// * The order of the boxes is the structure's, held to as they are handed
///   over, but for the `styp`, which the mux FSM lays down first if at all:
///   the `styp`, then the fragments. A `styp` handed over after another box is
///   [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder), and a
///   segment declared over without a fragment is
///   [`MissingMandatoryBox`](crate::ErrorKind::MissingMandatoryBox).
/// * A `styp` listing a brand under which the `default-base-is-moof` every
///   `tfhd` states shall not be used
///   ([`SegmentTypeBox::forbids_default_base_is_moof`]) is
///   [`UnsupportedBrand`](crate::ErrorKind::UnsupportedBrand), and nothing
///   of it is laid down. The `ftyp` of the initialization segment is not
///   handed over, and is not checked.
/// * A fragment is opened by [`begin_fragment`](Self::begin_fragment) or
///   [`begin_fragment_continuing`](Self::begin_fragment_continuing),
///   carries the samples handed over next, and is laid down by
///   [`finish_fragment`](Self::finish_fragment) as the `moof` and the `mdat`
///   the sample layer made of it. What the samples themselves must hold to
///   is [`MovieFragmentWriter`]'s contract, reported as
///   [`Sample`](crate::ErrorKind::Sample): the samples are checked against
///   the movie of the initialization segment, and a movie that continues in
///   no fragments is refused by [`new`](Self::new). A segment written apart
///   from the ones before it starts each track where its first sample
///   states, since every fragment states a `tfdt`, or at zero where the first
///   fragment carrying the track was opened by
///   [`begin_fragment_continuing`](Self::begin_fragment_continuing).
/// * The bytes are taken from [`poll_output`](Self::poll_output), one
///   [`EventBytes`] a call, owned by whoever takes them. The caller drains
///   before handing over more: bytes are held until they are taken, so
///   writing on without polling has the mux FSM hold the whole segment.
/// * An `Err` leaves the mux FSM failed for good,
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished) aside:
///   every later call reports that same failure again. The bytes made before
///   it are still there to take.
/// * [`finish`](Self::finish) declares the segment over. Bytes are still
///   taken after it, but anything handed over then, or a second
///   [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished).
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{SampleFlags, TrackExtendsBox};
/// use isobmff_sample::Sample;
/// use isobmff_structure::MediaSegmentMuxFsm;
/// # use isobmff_test_support::{fragmented_movie, segment_type};
/// // A segment continuing the movie of track 1, opening with its brands
/// let movie = fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO));
/// let mut mux_fsm = MediaSegmentMuxFsm::new(&movie)?;
/// mux_fsm.handle_segment_type(segment_type())?;
///
/// // One fragment of two samples of track 1, lasting 1024 units each
/// mux_fsm.begin_fragment(1)?;
/// mux_fsm.handle_sample(Sample::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec()))?;
/// mux_fsm.handle_sample(Sample::new(1, 1_024, 1_024, 0, SampleFlags::ZERO, 1, b"DATA".to_vec()))?;
/// mux_fsm.finish_fragment()?;
/// mux_fsm.finish()?;
///
/// // The bytes are drained as the mux FSM hands them over
/// let mut segment = Vec::new();
/// while let Some(written) = mux_fsm.poll_output() {
///     segment.extend_from_slice(&written);
/// }
///
/// // The segment opens with the brands, and the media data holds the samples end to end
/// assert_eq!(&segment[4..8], b"styp");
/// assert!(segment.ends_with(b"SAMPDATA"));
/// # Ok::<(), isobmff_structure::Error>(())
/// ```
#[derive(Debug)]
pub struct MediaSegmentMuxFsm {
    output: MuxOutput,
    structure: MediaSegmentStructure,
    samples: MovieFragmentWriter,
}

impl MediaSegmentMuxFsm {
    /// Creates a mux FSM waiting at the start of a media segment that continues `movie`
    ///
    /// `movie` is the movie of the initialization segment, which the samples
    /// are checked against.
    ///
    /// # Errors
    ///
    /// * [`Sample`](crate::ErrorKind::Sample): the movie is one
    ///   [`MovieFragmentWriter::new`] refuses — it carries no `mvex`, or its
    ///   sample tables lay samples out.
    pub fn new(movie: &MovieBox) -> Result<Self, Error> {
        Ok(Self {
            output: MuxOutput::new(),
            structure: MediaSegmentStructure::new(),
            samples: MovieFragmentWriter::new(movie)?,
        })
    }

    /// Takes the brands the segment declares itself readable as, and lays them down
    ///
    /// # Errors
    ///
    /// * [`UnsupportedBrand`](crate::ErrorKind::UnsupportedBrand): a
    ///   brand listed forbids the `default-base-is-moof` the mux FSM writes;
    ///   nothing of the box is laid down.
    /// * [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder): a box
    ///   was laid down before them.
    /// * [`Box`](crate::ErrorKind::Box): the box does not write.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   segment was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the mux FSM keeps and reports
    ///   again for every call after it.
    pub fn handle_segment_type(&mut self, segment_type: SegmentTypeBox) -> Result<(), Error> {
        self.output.writing()?;
        let mut lay_down_segment_type = || -> Result<(), Error> {
            if segment_type.forbids_default_base_is_moof() {
                return Err(Error::unsupported_brand());
            }
            if !self.structure.is_at_start() {
                return Err(Error::box_out_of_order(SegmentTypeBox::BOX_TYPE));
            }
            self.write_value(&segment_type)
        };
        let laid_down = lay_down_segment_type();

        self.output.record(laid_down)
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
    ///   segment was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the mux FSM keeps and reports
    ///   again for every call after it.
    pub fn begin_fragment(&mut self, sequence_number: u32) -> Result<(), Error> {
        self.output.writing()?;
        let begun = self
            .samples
            .begin_fragment(sequence_number)
            .map_err(Error::from);

        self.output.record(begun)
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
    ///   segment was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the mux FSM keeps and reports
    ///   again for every call after it.
    pub fn begin_fragment_continuing(&mut self, sequence_number: u32) -> Result<(), Error> {
        self.output.writing()?;
        let begun = self
            .samples
            .begin_fragment_continuing(sequence_number)
            .map_err(Error::from);

        self.output.record(begun)
    }

    /// Takes a sample, and places it in the fragment that is open
    ///
    /// # Errors
    ///
    /// * [`Sample`](crate::ErrorKind::Sample): what the sample layer
    ///   makes of the sample.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   segment was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the mux FSM keeps and reports
    ///   again for every call after it.
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), Error> {
        self.output.writing()?;
        let placed = self.samples.handle_sample(sample).map_err(Error::from);

        self.output.record(placed)
    }

    /// Closes the fragment that is open, and lays it down
    ///
    /// The `moof` and the `mdat` the sample layer made of it are written here,
    /// the media data moving into the segment rather than being copied into
    /// it.
    ///
    /// # Errors
    ///
    /// * [`Sample`](crate::ErrorKind::Sample): what the sample layer
    ///   makes of the fragment.
    /// * [`Box`](crate::ErrorKind::Box): the `moof` or the `mdat`
    ///   does not write.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   segment was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the mux FSM keeps and reports
    ///   again for every call after it.
    pub fn finish_fragment(&mut self) -> Result<(), Error> {
        self.output.writing()?;
        let mut lay_down_fragment = || -> Result<(), Error> {
            let (movie_fragment, media_data) = self.samples.finish_fragment()?;

            self.write_value(&movie_fragment)?;
            self.lay_down(MediaDataBox::BOX_TYPE, media_data)
        };
        let laid_down = lay_down_fragment();

        self.output.record(laid_down)
    }

    /// Hands over the bytes the segment has been laid down as so far
    ///
    /// Reports `None` once they are used up: more samples are needed, or the
    /// segment is over. Failure is reported by the calls that take the brands
    /// and the samples, so this one never fails — a failed mux FSM hands over
    /// the bytes it had already made, then nothing from there on.
    pub fn poll_output(&mut self) -> Option<EventBytes> {
        self.output.poll_output()
    }

    /// Declares the segment over
    ///
    /// # Errors
    ///
    /// * [`Sample`](crate::ErrorKind::Sample): a fragment was left
    ///   open.
    /// * [`MissingMandatoryBox`](crate::ErrorKind::MissingMandatoryBox):
    ///   no fragment was laid down, so what was laid down is not a media
    ///   segment.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   segment was already declared over.
    /// * The failure of a previous call, which the mux FSM keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.output.writing()?;
        let mut finish_segment = || -> Result<(), Error> {
            self.samples.finish()?;
            self.structure.finish()?;
            self.output.finish()
        };
        let finished = finish_segment();

        self.output.record(finished)
    }

    /// Lays `value` down as the whole box it forms
    fn write_value<Value: BoxEncode + BoxDefinition>(
        &mut self,
        value: &Value,
    ) -> Result<(), Error> {
        let payload = whole_payload(value)?;

        self.lay_down(Value::BOX_TYPE, payload)
    }

    /// Lays one box down where the structure places it, through the framing of the segment
    ///
    /// A box the structure passes over is refused as
    /// [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder).
    fn lay_down(&mut self, box_type: BoxType, payload: Vec<u8>) -> Result<(), Error> {
        let header = whole_box_header(box_type, payload.len() as u64)?;

        match self.structure.handle_box_type(box_type)? {
            MediaSegmentDisposition::SegmentType
            | MediaSegmentDisposition::MovieFragment
            | MediaSegmentDisposition::MediaData => {}
            MediaSegmentDisposition::SegmentIndex | MediaSegmentDisposition::Skip => {
                return Err(Error::box_out_of_order(box_type));
            }
        }

        self.output.frame(header, [payload])
    }
}

#[cfg(test)]
mod tests {
    use isobmff_boxes::{MovieFragmentBox, SampleFlags, SegmentTypeBox};
    use isobmff_core::{BoxDefinition, FourCC};
    use isobmff_sample::Sample;
    use isobmff_test_support::{segment_type, unfragmented_movie};

    use super::super::tests::{movie, sample};
    use super::{Error, MediaSegmentMuxFsm};
    use crate::ErrorKind;

    #[test]
    fn a_segment_declaring_no_brands_is_laid_down_all_the_same() {
        let mut mux_fsm = MediaSegmentMuxFsm::new(&movie()).unwrap();

        mux_fsm.begin_fragment(1).unwrap();
        mux_fsm.finish_fragment().unwrap();

        assert_eq!(mux_fsm.finish(), Ok(()));
        assert!(mux_fsm.poll_output().unwrap().ends_with(b"moof"));
    }

    #[test]
    fn brands_forbidding_default_base_is_moof_are_rejected_before_anything_is_laid_down() {
        let mut mux_fsm = MediaSegmentMuxFsm::new(&movie()).unwrap();

        assert_eq!(
            mux_fsm.handle_segment_type(SegmentTypeBox::new(
                FourCC::new(*b"msdh"),
                0,
                alloc::vec![FourCC::new(*b"msdh"), FourCC::new(*b"isom")],
            )),
            Err(Error::unsupported_brand())
        );
        assert_eq!(mux_fsm.poll_output(), None);
    }

    #[test]
    fn brands_handed_over_twice_are_rejected() {
        let mut mux_fsm = MediaSegmentMuxFsm::new(&movie()).unwrap();

        mux_fsm.handle_segment_type(segment_type()).unwrap();

        assert_eq!(
            mux_fsm.handle_segment_type(segment_type()),
            Err(Error::box_out_of_order(SegmentTypeBox::BOX_TYPE))
        );
    }

    #[test]
    fn brands_handed_over_after_a_fragment_are_rejected() {
        let mut mux_fsm = MediaSegmentMuxFsm::new(&movie()).unwrap();

        mux_fsm.begin_fragment(1).unwrap();
        mux_fsm.finish_fragment().unwrap();

        assert_eq!(
            mux_fsm.handle_segment_type(segment_type()),
            Err(Error::box_out_of_order(SegmentTypeBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_sample_handed_over_while_no_fragment_is_open_is_rejected() {
        let mut mux_fsm = MediaSegmentMuxFsm::new(&movie()).unwrap();

        assert!(matches!(
            mux_fsm.handle_sample(sample()).map_err(Error::kind),
            Err(ErrorKind::Sample(
                isobmff_sample::Error::NoFragmentOpen { .. }
            ))
        ));
    }

    #[test]
    fn a_mux_fsm_is_made_only_for_a_movie_continued_in_fragments() {
        assert!(matches!(
            MediaSegmentMuxFsm::new(&unfragmented_movie())
                .map(|_mux_fsm| ())
                .map_err(Error::kind),
            Err(ErrorKind::Sample(
                isobmff_sample::Error::MissingMovieExtends { .. }
            ))
        ));
    }

    #[test]
    fn a_sample_of_a_track_the_movie_of_the_initialization_segment_does_not_declare_is_rejected() {
        let mut mux_fsm = MediaSegmentMuxFsm::new(&movie()).unwrap();

        mux_fsm.begin_fragment(1).unwrap();

        assert!(matches!(
            mux_fsm
                .handle_sample(Sample::new(
                    999,
                    0,
                    1_024,
                    0,
                    SampleFlags::ZERO,
                    1,
                    b"SAMP".to_vec()
                ))
                .map_err(Error::kind),
            Err(ErrorKind::Sample(isobmff_sample::Error::UnknownTrackId {
                track_id: 999,
                ..
            }))
        ));
    }

    #[test]
    fn a_segment_declared_over_without_a_fragment_is_rejected() {
        let mut mux_fsm = MediaSegmentMuxFsm::new(&movie()).unwrap();

        mux_fsm.handle_segment_type(segment_type()).unwrap();

        assert_eq!(
            mux_fsm.finish(),
            Err(Error::missing_mandatory_box(MovieFragmentBox::BOX_TYPE))
        );
    }

    #[test]
    fn anything_handed_over_after_finishing_is_rejected() {
        let mut mux_fsm = MediaSegmentMuxFsm::new(&movie()).unwrap();

        mux_fsm.begin_fragment(1).unwrap();
        mux_fsm.finish_fragment().unwrap();
        mux_fsm.finish().unwrap();

        assert_eq!(
            mux_fsm.handle_sample(sample()),
            Err(Error::already_finished())
        );
        assert_eq!(mux_fsm.finish(), Err(Error::already_finished()));
    }
}
