//! [`FragmentedMuxFsm`], a fragmented movie file laid down as the samples come

use alloc::vec::Vec;

use isobmff_boxes::{FileTypeBox, MediaDataBox, MovieBox, MovieFragmentBox};
use isobmff_core::{BoxDefinition, BoxEncode, BoxType, FourCC};
use isobmff_sample::{MovieFragmentWriter, Sample};
use isobmff_sequence::EventBytes;

use super::{FragmentedDisposition, FragmentedStructure};
use crate::mux_output::MuxOutput;
use crate::{Error, whole_box_header, whole_payload};

/// Lays a fragmented movie file down, taking the samples as they come
///
/// The mirror of [`MovieDemuxFsm`](crate::MovieDemuxFsm):
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
///   [`BoxOutOfOrder`](crate::Error::BoxOutOfOrder) or
///   [`DuplicateBox`](crate::Error::DuplicateBox), and a file
///   declared over without a `moov` is
///   [`MissingMandatoryBox`](crate::Error::MissingMandatoryBox).
/// * The movie comes before any fragment: a fragment opened or closed, or a
///   sample handed over, before it is
///   [`BoxOutOfOrder`](crate::Error::BoxOutOfOrder) of the `moof`. The
///   samples are checked against the movie as they are handed over, as
///   [`MovieFragmentWriter`] checks them, and a movie it refuses is refused
///   before any of it is laid down.
/// * The `ftyp` handed over is laid down as it stands, unless it lists a
///   brand under which the `default-base-is-moof` every `tfhd` states shall
///   not be used ([`FileTypeBox::forbids_default_base_is_moof`]):
///   that is [`UnsupportedBrand`](crate::Error::UnsupportedBrand), and
///   nothing of it is laid down. Where none was handed over, the mux FSM
///   lays its own down before the `moov`: `iso6` as its
///   `major_brand` and its one `compatible_brands` entry, with
///   `minor_version` 0, the brand the widest layout it lays down requires
///   (§8.8.7.1, Annex E.9).
/// * A fragment is opened by [`begin_fragment`](Self::begin_fragment) or
///   [`begin_fragment_continuing`](Self::begin_fragment_continuing),
///   carries the samples handed over next, and is laid down by
///   [`finish_fragment`](Self::finish_fragment) as the `moof` and the `mdat`
///   the sample layer made of it. What the samples themselves must hold to
///   is [`MovieFragmentWriter`]'s contract, reported as
///   [`Sample`](crate::Error::Sample).
/// * The bytes are taken from [`poll_output`](Self::poll_output), one
///   [`EventBytes`] a call, owned by whoever takes them. The caller drains
///   before handing over more: bytes are held until they are taken, so
///   writing on without polling has the mux FSM hold the whole file.
/// * An `Err` leaves the mux FSM failed for good,
///   [`AlreadyFinished`](crate::Error::AlreadyFinished) aside:
///   every later call reports that same failure again. The bytes made before
///   it are still there to take.
/// * [`finish`](Self::finish) declares the file over. Bytes are still taken
///   after it, but anything handed over then, or a second
///   [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::Error::AlreadyFinished).
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{SampleFlags, TrackExtendsBox};
/// use isobmff_sample::Sample;
/// use isobmff_structure::FragmentedMuxFsm;
/// # use isobmff_test_support::fragmented_movie;
/// // A file handed no brands, only the movie its fragments continue
/// let mut mux_fsm = FragmentedMuxFsm::new();
/// mux_fsm.handle_movie(fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO)))?;
///
/// // One fragment of two samples of track 1, lasting 1024 units each
/// mux_fsm.begin_fragment(1)?;
/// mux_fsm.handle_sample(Sample::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec()))?;
/// mux_fsm.handle_sample(Sample::new(1, 1_024, 1_024, 0, SampleFlags::ZERO, 1, b"DATA".to_vec()))?;
/// mux_fsm.finish_fragment()?;
/// mux_fsm.finish()?;
///
/// // The bytes are drained as the mux FSM hands them over
/// let mut file = Vec::new();
/// while let Some(written) = mux_fsm.poll_output() {
///     file.extend_from_slice(&written);
/// }
///
/// // The file opens with the brands the mux FSM declares, and the media data holds the samples end to end
/// assert_eq!(&file[4..8], b"ftyp");
/// assert!(file.ends_with(b"SAMPDATA"));
/// # Ok::<(), isobmff_structure::Error>(())
/// ```
#[derive(Debug)]
pub struct FragmentedMuxFsm {
    output: MuxOutput,
    structure: FragmentedStructure,
    samples: Option<MovieFragmentWriter>,
}

impl FragmentedMuxFsm {
    /// Creates a mux FSM waiting at the start of a fragmented movie file
    #[must_use]
    pub const fn new() -> Self {
        Self {
            output: MuxOutput::new(),
            structure: FragmentedStructure::new(),
            samples: None,
        }
    }

    /// Takes the brands the file declares itself readable as, and lays them down
    ///
    /// # Errors
    ///
    /// * [`UnsupportedBrand`](crate::Error::UnsupportedBrand): a
    ///   brand listed forbids the `default-base-is-moof` the mux FSM writes;
    ///   nothing of the box is laid down.
    /// * [`BoxOutOfOrder`](crate::Error::BoxOutOfOrder): a box
    ///   was handed over before them.
    /// * [`Box`](crate::Error::Box): the box does not write.
    /// * [`AlreadyFinished`](crate::Error::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the mux FSM keeps and reports
    ///   again for every call after it.
    pub fn handle_file_type(&mut self, file_type: FileTypeBox) -> Result<(), Error> {
        self.output.writing()?;
        let mut lay_down_file_type = || -> Result<(), Error> {
            if file_type.forbids_default_base_is_moof() {
                return Err(Error::UnsupportedBrand);
            }
            self.write_value(&file_type)
        };
        let laid_down = lay_down_file_type();

        self.output.record(laid_down)
    }

    /// Takes the movie the fragments continue, and lays it down
    ///
    /// The samples handed over from here on are checked against it.
    ///
    /// # Errors
    ///
    /// * [`Sample`](crate::Error::Sample): the movie is one
    ///   [`MovieFragmentWriter::new`] refuses — it carries no `mvex`, or its
    ///   sample tables lay samples out; nothing of it is laid down.
    /// * [`DuplicateBox`](crate::Error::DuplicateBox): a movie
    ///   continuing in fragments was handed over already.
    /// * [`Box`](crate::Error::Box): the box does not write.
    /// * [`AlreadyFinished`](crate::Error::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the mux FSM keeps and reports
    ///   again for every call after it.
    pub fn handle_movie(&mut self, movie: MovieBox) -> Result<(), Error> {
        self.output.writing()?;
        let mut lay_down_movie = || -> Result<(), Error> {
            let samples = MovieFragmentWriter::new(&movie)?;
            if self.structure.is_at_start() {
                self.write_value(&default_file_type())?;
            }
            self.write_value(&movie)?;
            self.samples = Some(samples);

            Ok(())
        };
        let laid_down = lay_down_movie();

        self.output.record(laid_down)
    }

    /// Opens a fragment, which the samples handed over next are laid out in
    ///
    /// `sequence_number` is what its `mfhd` states, which §8.8.5 has increase
    /// over the fragments of a presentation.
    ///
    /// # Errors
    ///
    /// * [`BoxOutOfOrder`](crate::Error::BoxOutOfOrder): the
    ///   movie was not handed over first.
    /// * [`Sample`](crate::Error::Sample): what the sample layer
    ///   makes of the call.
    /// * [`AlreadyFinished`](crate::Error::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the mux FSM keeps and reports
    ///   again for every call after it.
    pub fn begin_fragment(&mut self, sequence_number: u32) -> Result<(), Error> {
        self.output.writing()?;
        let begun = self
            .samples()
            .and_then(|samples| samples.begin_fragment(sequence_number).map_err(Error::from));

        self.output.record(begun)
    }

    /// Opens a fragment in which every track continues where the samples written for it reach, as [`MovieFragmentWriter::begin_fragment_continuing`] places them
    ///
    /// `sequence_number` is what its `mfhd` states, as for
    /// [`begin_fragment`](Self::begin_fragment).
    ///
    /// # Errors
    ///
    /// * [`BoxOutOfOrder`](crate::Error::BoxOutOfOrder): the
    ///   movie was not handed over first.
    /// * [`Sample`](crate::Error::Sample): what the sample layer
    ///   makes of the call.
    /// * [`AlreadyFinished`](crate::Error::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the mux FSM keeps and reports
    ///   again for every call after it.
    pub fn begin_fragment_continuing(&mut self, sequence_number: u32) -> Result<(), Error> {
        self.output.writing()?;
        let begun = self.samples().and_then(|samples| {
            samples
                .begin_fragment_continuing(sequence_number)
                .map_err(Error::from)
        });

        self.output.record(begun)
    }

    /// Takes a sample, and places it in the fragment that is open
    ///
    /// # Errors
    ///
    /// * [`BoxOutOfOrder`](crate::Error::BoxOutOfOrder): the
    ///   movie was not handed over first.
    /// * [`Sample`](crate::Error::Sample): what the sample layer
    ///   makes of the sample.
    /// * [`AlreadyFinished`](crate::Error::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the mux FSM keeps and reports
    ///   again for every call after it.
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), Error> {
        self.output.writing()?;
        let placed = self
            .samples()
            .and_then(|samples| samples.handle_sample(sample).map_err(Error::from));

        self.output.record(placed)
    }

    /// Closes the fragment that is open, and lays it down
    ///
    /// The `moof` and the `mdat` the sample layer made of it are written here,
    /// the media data moving into the file rather than being copied into it.
    ///
    /// # Errors
    ///
    /// * [`BoxOutOfOrder`](crate::Error::BoxOutOfOrder): the
    ///   movie was not handed over first.
    /// * [`Sample`](crate::Error::Sample): what the sample layer
    ///   makes of the fragment.
    /// * [`Box`](crate::Error::Box): the `moof` or the `mdat`
    ///   does not write.
    /// * [`AlreadyFinished`](crate::Error::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the mux FSM keeps and reports
    ///   again for every call after it.
    pub fn finish_fragment(&mut self) -> Result<(), Error> {
        self.output.writing()?;
        let mut lay_down_fragment = || -> Result<(), Error> {
            let (movie_fragment, media_data) = self.samples()?.finish_fragment()?;

            self.write_value(&movie_fragment)?;
            self.lay_down(MediaDataBox::BOX_TYPE, media_data)
        };
        let laid_down = lay_down_fragment();

        self.output.record(laid_down)
    }

    /// Hands over the bytes the file has been laid down as so far
    ///
    /// Reports `None` once they are used up: more samples are needed, or the
    /// file is over. Failure is reported by the calls that take the boxes and
    /// the samples, so this one never fails — a failed mux FSM hands over the
    /// bytes it had already made, then nothing from there on.
    pub fn poll_output(&mut self) -> Option<EventBytes> {
        self.output.poll_output()
    }

    /// Declares the file over
    ///
    /// # Errors
    ///
    /// * [`Sample`](crate::Error::Sample): a fragment was left
    ///   open.
    /// * [`MissingMandatoryBox`](crate::Error::MissingMandatoryBox):
    ///   the movie was never handed over, so the file laid down is not a
    ///   fragmented movie file.
    /// * [`AlreadyFinished`](crate::Error::AlreadyFinished): the
    ///   file was already declared over.
    /// * The failure of a previous call, which the mux FSM keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.output.writing()?;
        let mut finish_file = || -> Result<(), Error> {
            if let Some(samples) = &mut self.samples {
                samples.finish()?;
            }
            self.structure.finish()?;
            self.output.finish()
        };
        let finished = finish_file();

        self.output.record(finished)
    }

    /// Returns the sample layer, made once the movie was handed over
    fn samples(&mut self) -> Result<&mut MovieFragmentWriter, Error> {
        self.samples.as_mut().ok_or(Error::BoxOutOfOrder {
            box_type: MovieFragmentBox::BOX_TYPE,
        })
    }

    /// Lays `value` down as the whole box it forms
    fn write_value<Value: BoxEncode + BoxDefinition>(
        &mut self,
        value: &Value,
    ) -> Result<(), Error> {
        let payload = whole_payload(value)?;

        self.lay_down(Value::BOX_TYPE, payload)
    }

    /// Lays one box down where the structure places it, through the framing of the file
    ///
    /// A box the structure passes over is refused as
    /// [`BoxOutOfOrder`](crate::Error::BoxOutOfOrder).
    fn lay_down(&mut self, box_type: BoxType, payload: Vec<u8>) -> Result<(), Error> {
        let header = whole_box_header(box_type, payload.len() as u64)?;

        match self.structure.handle_box_type(box_type)? {
            FragmentedDisposition::FileType
            | FragmentedDisposition::Movie
            | FragmentedDisposition::MovieFragment
            | FragmentedDisposition::MediaData => {}
            FragmentedDisposition::SegmentIndex
            | FragmentedDisposition::MovieFragmentRandomAccess
            | FragmentedDisposition::Skip => {
                return Err(Error::BoxOutOfOrder { box_type });
            }
        }

        self.output.frame(header, [payload])
    }
}

impl Default for FragmentedMuxFsm {
    fn default() -> Self {
        Self::new()
    }
}

/// Brands the mux FSM declares where none were handed over, those the widest layout it lays down requires
fn default_file_type() -> FileTypeBox {
    FileTypeBox::new(FourCC::new(*b"iso6"), 0, alloc::vec![FourCC::new(*b"iso6")])
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use isobmff_boxes::{FileTypeBox, MovieBox, MovieFragmentBox, SampleFlags, TrackExtendsBox};
    use isobmff_core::{BoxDecode, BoxDefinition, FourCC};
    use isobmff_sample::Sample;
    use isobmff_test_support::{file_type, fragmented_movie, unfragmented_movie};

    use super::{Error, FragmentedMuxFsm, default_file_type};

    /// Movie of one track continued in fragments, whose defaults a `trex` states
    fn movie() -> MovieBox {
        fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO))
    }

    /// A sample of the track the movie declares
    fn sample() -> Sample {
        Sample::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec())
    }

    /// The bytes the mux FSM has laid down, drained to the end
    fn drained(mux_fsm: &mut FragmentedMuxFsm) -> Vec<u8> {
        let mut file = Vec::new();
        while let Some(written) = mux_fsm.poll_output() {
            file.extend_from_slice(&written);
        }

        file
    }

    #[test]
    fn a_file_handed_no_brands_opens_with_the_brands_the_mux_fsm_declares() {
        let mut mux_fsm = FragmentedMuxFsm::new();

        mux_fsm.handle_movie(movie()).unwrap();
        mux_fsm.finish().unwrap();
        let file = drained(&mut mux_fsm);

        assert_eq!(
            FileTypeBox::decode(&file).map(|(file_type, rest)| (file_type, rest.get(4..8))),
            Ok((default_file_type(), Some(b"moov".as_slice())))
        );
    }

    #[test]
    fn the_brands_handed_over_are_laid_down_as_they_stand() {
        let mut mux_fsm = FragmentedMuxFsm::new();

        mux_fsm.handle_file_type(file_type()).unwrap();
        mux_fsm.handle_movie(movie()).unwrap();
        mux_fsm.finish().unwrap();
        let file = drained(&mut mux_fsm);

        assert_eq!(
            FileTypeBox::decode(&file).map(|(file_type, rest)| (file_type, rest.get(4..8))),
            Ok((file_type(), Some(b"moov".as_slice())))
        );
    }

    #[test]
    fn brands_forbidding_default_base_is_moof_are_rejected_before_anything_is_laid_down() {
        let mut mux_fsm = FragmentedMuxFsm::new();

        assert_eq!(
            mux_fsm.handle_file_type(FileTypeBox::new(
                FourCC::new(*b"iso6"),
                0,
                alloc::vec![FourCC::new(*b"iso6"), FourCC::new(*b"isom")],
            )),
            Err(Error::UnsupportedBrand)
        );
        assert_eq!(mux_fsm.poll_output(), None);
    }

    #[test]
    fn a_fragment_opened_or_a_sample_handed_over_before_the_movie_is_rejected() {
        let mut opened = FragmentedMuxFsm::new();
        let mut handed_a_sample = FragmentedMuxFsm::new();

        opened.handle_file_type(file_type()).unwrap();

        assert_eq!(
            opened.begin_fragment(1),
            Err(Error::BoxOutOfOrder {
                box_type: MovieFragmentBox::BOX_TYPE
            })
        );
        assert_eq!(
            handed_a_sample.handle_sample(sample()),
            Err(Error::BoxOutOfOrder {
                box_type: MovieFragmentBox::BOX_TYPE
            })
        );
    }

    #[test]
    fn a_movie_continued_in_no_fragments_is_rejected_before_anything_is_laid_down() {
        let mut mux_fsm = FragmentedMuxFsm::new();

        assert!(matches!(
            mux_fsm.handle_movie(unfragmented_movie()),
            Err(Error::Sample {
                error: isobmff_sample::Error::MissingMovieExtends { .. }
            })
        ));
        assert_eq!(mux_fsm.poll_output(), None);
    }

    #[test]
    fn a_sample_the_movie_does_not_resolve_is_rejected_where_it_is_handed_over() {
        let refused = |sample: Sample| {
            let mut mux_fsm = FragmentedMuxFsm::new();
            mux_fsm.handle_movie(movie()).unwrap();
            mux_fsm.begin_fragment(1).unwrap();

            mux_fsm.handle_sample(sample)
        };

        assert!(matches!(
            refused(Sample::new(
                999,
                0,
                1_024,
                0,
                SampleFlags::ZERO,
                1,
                b"SAMP".to_vec()
            )),
            Err(Error::Sample {
                error: isobmff_sample::Error::UnknownTrackId { track_id: 999, .. }
            })
        ));
        assert!(matches!(
            refused(Sample::new(
                1,
                0,
                1_024,
                0,
                SampleFlags::ZERO,
                2,
                b"SAMP".to_vec()
            )),
            Err(Error::Sample {
                error: isobmff_sample::Error::UnknownSampleDescriptionIndex {
                    track_id: 1,
                    sample_description_index: 2,
                    ..
                }
            })
        ));
    }

    #[test]
    fn a_sample_handed_over_while_no_fragment_is_open_is_rejected() {
        let mut mux_fsm = FragmentedMuxFsm::new();

        mux_fsm.handle_movie(movie()).unwrap();

        assert!(matches!(
            mux_fsm.handle_sample(sample()),
            Err(Error::Sample {
                error: isobmff_sample::Error::NoFragmentOpen { .. }
            })
        ));
    }

    #[test]
    fn a_file_declared_over_without_a_movie_is_rejected() {
        let mut mux_fsm = FragmentedMuxFsm::new();

        mux_fsm.handle_file_type(file_type()).unwrap();

        assert_eq!(
            mux_fsm.finish(),
            Err(Error::MissingMandatoryBox {
                box_type: MovieBox::BOX_TYPE
            })
        );
    }

    #[test]
    fn anything_handed_over_after_finishing_is_rejected() {
        let mut mux_fsm = FragmentedMuxFsm::new();

        mux_fsm.handle_file_type(file_type()).unwrap();
        mux_fsm.handle_movie(movie()).unwrap();
        mux_fsm.finish().unwrap();

        assert_eq!(mux_fsm.handle_sample(sample()), Err(Error::AlreadyFinished));
        assert_eq!(mux_fsm.finish(), Err(Error::AlreadyFinished));
    }
}
