//! [`MediaSegmentMuxer`], a media segment laid down as the samples come, ISO/IEC 14496-12 §8.16

use alloc::vec::Vec;

use isobmff_boxes::{MediaDataBox, MovieBox, SegmentTypeBox};
use isobmff_core::{BoxDefinition, BoxEncode, BoxType};

use crate::mux::{MuxError, MuxOutput, whole_box_header, whole_payload};
use crate::sample::{MovieFragmentWriter, Sample};
use crate::sequence::OutputBytes;
use crate::structure::{MediaSegmentDisposition, MediaSegmentStructure};

/// Lays a media segment down, taking the samples as they come
///
/// The mirror of [`MediaSegmentDemuxer`](crate::MediaSegmentDemuxer): it wires
/// the layers that write a media segment of ISO/IEC 14496-12 §8.16 — the
/// structure that holds the order of the top-level boxes, the writing of
/// each box whole, the laying out of the samples of a fragment as its `moof`
/// and the media data beside it, and the framing of the segment — so a
/// caller hands over brands and samples and takes bytes. The movie the
/// segment continues — that of the initialization segment — is taken when
/// the muxer is made, and not written: a segment carries none. Beside the
/// structure it holds two rules of its own: a `styp` comes first if at all,
/// and none listing a brand that forbids the `default-base-is-moof` it writes
/// is laid down. It reaches for no destination: when to write and to where
/// stay with the caller.
///
/// # Contract
///
/// * The order of the boxes is the structure's, held to as they are handed
///   over, but for the `styp`, which the muxer lays down first if at all:
///   the `styp`, then the fragments. A `styp` handed over after another box is
///   [`BoxOutOfOrder`](crate::MuxError::BoxOutOfOrder), and a
///   segment declared over without a fragment is
///   [`MissingMandatoryBox`](crate::MuxError::MissingMandatoryBox).
/// * A `styp` listing a brand under which the `default-base-is-moof` every
///   `tfhd` states shall not be used
///   ([`SegmentTypeBox::forbids_default_base_is_moof`]) is
///   [`UnsupportedBrand`](crate::MuxError::UnsupportedBrand), and nothing
///   of it is laid down. The `ftyp` of the initialization segment is not
///   handed over, and is not checked.
/// * A fragment is opened by [`begin_fragment`](Self::begin_fragment) or
///   [`begin_fragment_continuing`](Self::begin_fragment_continuing),
///   carries the samples handed over next, and is laid down by
///   [`finish_fragment`](Self::finish_fragment) as the `moof` and the `mdat`
///   the sample layer made of it. What the samples themselves must hold to
///   is [`MovieFragmentWriter`]'s contract, reported as
///   [`Sample`](crate::MuxError::Sample): the samples are checked against
///   the movie of the initialization segment, and a movie that continues in
///   no fragments is refused by [`new`](Self::new). A segment written apart
///   from the ones before it starts each track where its first sample
///   states, since every fragment states a `tfdt`, or at zero where the first
///   fragment carrying the track was opened by
///   [`begin_fragment_continuing`](Self::begin_fragment_continuing).
/// * The bytes are taken from [`poll_output`](Self::poll_output), one
///   [`OutputBytes`] a call, owned by whoever takes them. The caller drains
///   before handing over more: bytes are held until they are taken, so
///   writing on without polling has the muxer hold the whole segment.
/// * An `Err` leaves the muxer failed for good,
///   [`AlreadyFinished`](crate::MuxError::AlreadyFinished) aside:
///   every later call reports that same failure again. The bytes made before
///   it are still there to take.
/// * [`finish`](Self::finish) declares the segment over. Bytes are still
///   taken after it, but anything handed over then, or a second
///   [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::MuxError::AlreadyFinished).
///
/// # Examples
///
/// ```
/// use isobmff::boxes::{SampleFlags, TrackExtendsBox};
/// use isobmff::sample::{Sample, SampleProperties};
/// use isobmff::MediaSegmentMuxer;
/// # use isobmff_test_support::{fragmented_movie, segment_type};
/// // A segment continuing the movie of track 1, opening with its brands
/// let movie = fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO));
/// let mut muxer = MediaSegmentMuxer::new(&movie)?;
/// muxer.handle_segment_type(segment_type())?;
///
/// // One fragment of two samples of track 1, lasting 1024 units each
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
/// // The bytes are drained as the muxer hands them over
/// let mut segment = Vec::new();
/// while let Some(written) = muxer.poll_output() {
///     segment.extend_from_slice(&written);
/// }
///
/// // The segment opens with the brands, and the media data holds the samples end to end
/// assert_eq!(&segment[4..8], b"styp");
/// assert!(segment.ends_with(b"SAMPDATA"));
/// # Ok::<(), isobmff::MuxError>(())
/// ```
#[derive(Debug)]
pub struct MediaSegmentMuxer {
    output: MuxOutput,
    structure: MediaSegmentStructure,
    samples: MovieFragmentWriter,
}

impl MediaSegmentMuxer {
    /// Creates a muxer waiting at the start of a media segment that continues `movie`
    ///
    /// `movie` is the movie of the initialization segment, which the samples
    /// are checked against.
    ///
    /// # Errors
    ///
    /// * [`Sample`](crate::MuxError::Sample): the movie is one
    ///   [`MovieFragmentWriter::new`] refuses — it carries no `mvex`, or its
    ///   sample tables lay samples out.
    pub fn new(movie: &MovieBox) -> Result<Self, MuxError> {
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
    /// * [`UnsupportedBrand`](crate::MuxError::UnsupportedBrand): a
    ///   brand listed forbids the `default-base-is-moof` the muxer writes;
    ///   nothing of the box is laid down.
    /// * [`BoxOutOfOrder`](crate::MuxError::BoxOutOfOrder): a box
    ///   was laid down before them.
    /// * [`Box`](crate::MuxError::Box): the box does not write.
    /// * [`AlreadyFinished`](crate::MuxError::AlreadyFinished): the
    ///   segment was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the muxer keeps and reports
    ///   again for every call after it.
    pub fn handle_segment_type(&mut self, segment_type: SegmentTypeBox) -> Result<(), MuxError> {
        self.output.writing()?;
        let mut write_segment_type = || -> Result<(), MuxError> {
            if segment_type.forbids_default_base_is_moof() {
                return Err(MuxError::UnsupportedBrand);
            }
            if !self.structure.is_at_start() {
                return Err(MuxError::BoxOutOfOrder {
                    box_type: SegmentTypeBox::BOX_TYPE,
                });
            }
            self.write_value(&segment_type)
        };
        let written = write_segment_type();

        self.output.record(written)
    }

    /// Opens a fragment, which the samples handed over next are laid out in
    ///
    /// `sequence_number` is what its `mfhd` states, which §8.8.5 has increase
    /// over the fragments of a presentation.
    ///
    /// # Errors
    ///
    /// * [`Sample`](crate::MuxError::Sample): what the sample layer
    ///   makes of the call.
    /// * [`AlreadyFinished`](crate::MuxError::AlreadyFinished): the
    ///   segment was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the muxer keeps and reports
    ///   again for every call after it.
    pub fn begin_fragment(&mut self, sequence_number: u32) -> Result<(), MuxError> {
        self.output.writing()?;
        let begun = self
            .samples
            .begin_fragment(sequence_number)
            .map_err(MuxError::from);

        self.output.record(begun)
    }

    /// Opens a fragment in which every track continues where the samples written for it reach, as [`MovieFragmentWriter::begin_fragment_continuing`] places them
    ///
    /// `sequence_number` is what its `mfhd` states, as for
    /// [`begin_fragment`](Self::begin_fragment).
    ///
    /// # Errors
    ///
    /// * [`Sample`](crate::MuxError::Sample): what the sample layer
    ///   makes of the call.
    /// * [`AlreadyFinished`](crate::MuxError::AlreadyFinished): the
    ///   segment was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the muxer keeps and reports
    ///   again for every call after it.
    pub fn begin_fragment_continuing(&mut self, sequence_number: u32) -> Result<(), MuxError> {
        self.output.writing()?;
        let begun = self
            .samples
            .begin_fragment_continuing(sequence_number)
            .map_err(MuxError::from);

        self.output.record(begun)
    }

    /// Takes a sample, and places it in the fragment that is open
    ///
    /// # Errors
    ///
    /// * [`Sample`](crate::MuxError::Sample): what the sample layer
    ///   makes of the sample.
    /// * [`AlreadyFinished`](crate::MuxError::AlreadyFinished): the
    ///   segment was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the muxer keeps and reports
    ///   again for every call after it.
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), MuxError> {
        self.output.writing()?;
        let placed = self.samples.handle_sample(sample).map_err(MuxError::from);

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
    /// * [`Sample`](crate::MuxError::Sample): what the sample layer
    ///   makes of the fragment.
    /// * [`Box`](crate::MuxError::Box): the `moof` or the `mdat`
    ///   does not write.
    /// * [`AlreadyFinished`](crate::MuxError::AlreadyFinished): the
    ///   segment was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the muxer keeps and reports
    ///   again for every call after it.
    pub fn finish_fragment(&mut self) -> Result<(), MuxError> {
        self.output.writing()?;
        let mut write_fragment = || -> Result<(), MuxError> {
            self.samples.finish_fragment()?;
            while let Some((movie_fragment, media_data)) = self.samples.poll_fragment() {
                self.write_value(&movie_fragment)?;
                self.write_box(MediaDataBox::BOX_TYPE, media_data)?;
            }

            Ok(())
        };
        let written = write_fragment();

        self.output.record(written)
    }

    /// Hands over the bytes the segment has been laid down as so far
    ///
    /// Reports `None` once they are used up: more samples are needed, or the
    /// segment is over. Failure is reported by the calls that take the brands
    /// and the samples, so this one never fails — a failed muxer hands over
    /// the bytes it had already made, then nothing from there on.
    pub fn poll_output(&mut self) -> Option<OutputBytes> {
        self.output.poll_output()
    }

    /// Declares the segment over
    ///
    /// # Errors
    ///
    /// * [`Sample`](crate::MuxError::Sample): a fragment was left
    ///   open.
    /// * [`MissingMandatoryBox`](crate::MuxError::MissingMandatoryBox):
    ///   no fragment was laid down, so what was laid down is not a media
    ///   segment.
    /// * [`AlreadyFinished`](crate::MuxError::AlreadyFinished): the
    ///   segment was already declared over.
    /// * The failure of a previous call, which the muxer keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<(), MuxError> {
        self.output.writing()?;
        let mut finish_segment = || -> Result<(), MuxError> {
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
    ) -> Result<(), MuxError> {
        let payload = whole_payload(value)?;

        self.write_box(Value::BOX_TYPE, payload)
    }

    /// Lays one box down where the structure places it, through the framing of the segment
    ///
    /// A box the structure passes over is refused as
    /// [`BoxOutOfOrder`](crate::MuxError::BoxOutOfOrder).
    fn write_box(&mut self, box_type: BoxType, payload: Vec<u8>) -> Result<(), MuxError> {
        let header = whole_box_header(box_type, payload.len() as u64)?;

        match self.structure.handle_box_type(box_type)? {
            MediaSegmentDisposition::SegmentType
            | MediaSegmentDisposition::MovieFragment
            | MediaSegmentDisposition::MediaData => {}
            MediaSegmentDisposition::SegmentIndex | MediaSegmentDisposition::Skip => {
                return Err(MuxError::BoxOutOfOrder { box_type });
            }
        }

        self.output.write_box(header, [payload])
    }
}

#[cfg(test)]
mod tests {
    use isobmff_boxes::{MovieFragmentBox, SampleFlags, SegmentTypeBox};
    use isobmff_core::{BoxDefinition, FourCc};
    use isobmff_test_support::{segment_type, unfragmented_movie};

    use super::{MediaSegmentMuxer, MuxError};
    use crate::mux::driving::{movie, sample};
    use crate::sample::{Sample, SampleProperties};

    #[test]
    fn a_segment_declaring_no_brands_is_laid_down_all_the_same() {
        let mut muxer = MediaSegmentMuxer::new(&movie()).unwrap();

        muxer.begin_fragment(1).unwrap();
        muxer.finish_fragment().unwrap();

        assert_eq!(muxer.finish(), Ok(()));
        assert!(muxer.poll_output().unwrap().ends_with(b"moof"));
    }

    #[test]
    fn brands_forbidding_default_base_is_moof_are_rejected_before_anything_is_laid_down() {
        let mut muxer = MediaSegmentMuxer::new(&movie()).unwrap();

        assert_eq!(
            muxer.handle_segment_type(SegmentTypeBox::new(
                FourCc::new(*b"msdh"),
                0,
                alloc::vec![FourCc::new(*b"msdh"), FourCc::new(*b"isom")],
            )),
            Err(MuxError::UnsupportedBrand)
        );
        assert_eq!(muxer.poll_output(), None);
    }

    #[test]
    fn brands_handed_over_twice_are_rejected() {
        let mut muxer = MediaSegmentMuxer::new(&movie()).unwrap();

        muxer.handle_segment_type(segment_type()).unwrap();

        assert_eq!(
            muxer.handle_segment_type(segment_type()),
            Err(MuxError::BoxOutOfOrder {
                box_type: SegmentTypeBox::BOX_TYPE
            })
        );
    }

    #[test]
    fn brands_handed_over_after_a_fragment_are_rejected() {
        let mut muxer = MediaSegmentMuxer::new(&movie()).unwrap();

        muxer.begin_fragment(1).unwrap();
        muxer.finish_fragment().unwrap();

        assert_eq!(
            muxer.handle_segment_type(segment_type()),
            Err(MuxError::BoxOutOfOrder {
                box_type: SegmentTypeBox::BOX_TYPE
            })
        );
    }

    #[test]
    fn a_sample_handed_over_while_no_fragment_is_open_is_rejected() {
        let mut muxer = MediaSegmentMuxer::new(&movie()).unwrap();

        assert!(matches!(
            muxer.handle_sample(sample()),
            Err(MuxError::Sample {
                error: crate::sample::Error::NoFragmentOpen
            })
        ));
    }

    #[test]
    fn a_muxer_is_made_only_for_a_movie_continued_in_fragments() {
        assert!(matches!(
            MediaSegmentMuxer::new(&unfragmented_movie()).map(|_muxer| ()),
            Err(MuxError::Sample {
                error: crate::sample::Error::MissingMovieExtends
            })
        ));
    }

    #[test]
    fn a_sample_of_a_track_the_movie_of_the_initialization_segment_does_not_declare_is_rejected() {
        let mut muxer = MediaSegmentMuxer::new(&movie()).unwrap();

        muxer.begin_fragment(1).unwrap();

        assert!(matches!(
            muxer.handle_sample(Sample::new(
                SampleProperties {
                    track_id: 999,
                    decode_time: 0,
                    sample_duration: 1_024,
                    sample_composition_time_offset: 0,
                    sample_flags: SampleFlags::ZERO,
                    sample_description_index: 1
                },
                b"SAMP".to_vec()
            )),
            Err(MuxError::Sample {
                error: crate::sample::Error::UnknownTrackId { track_id: 999, .. }
            })
        ));
    }

    #[test]
    fn a_segment_declared_over_without_a_fragment_is_rejected() {
        let mut muxer = MediaSegmentMuxer::new(&movie()).unwrap();

        muxer.handle_segment_type(segment_type()).unwrap();

        assert_eq!(
            muxer.finish(),
            Err(MuxError::MissingMandatoryBox {
                box_type: MovieFragmentBox::BOX_TYPE
            })
        );
    }

    #[test]
    fn anything_handed_over_after_finishing_is_rejected() {
        let mut muxer = MediaSegmentMuxer::new(&movie()).unwrap();

        muxer.begin_fragment(1).unwrap();
        muxer.finish_fragment().unwrap();
        muxer.finish().unwrap();

        assert_eq!(
            muxer.handle_sample(sample()),
            Err(MuxError::AlreadyFinished)
        );
        assert_eq!(muxer.finish(), Err(MuxError::AlreadyFinished));
    }
}
