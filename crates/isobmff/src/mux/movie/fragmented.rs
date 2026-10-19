//! [`FragmentedMuxer`], a fragmented movie file laid down as the samples come, ISO/IEC 14496-12 Annex A.8

use alloc::vec::Vec;

use isobmff_boxes::{FileTypeBox, MediaDataBox, MovieBox, MovieFragmentBox};
use isobmff_core::{BoxDefinition, BoxEncode, BoxType, FourCc};

use crate::mux::{MuxError, MuxOutput, whole_box_header, whole_payload};
use crate::sample::{MovieFragmentWriter, Sample};
use crate::sequence::OutputBytes;
use crate::structure::{FragmentedDisposition, FragmentedStructure};

/// Lays a fragmented movie file down, taking the samples as they come
///
/// The mirror of [`MovieDemuxer`](crate::MovieDemuxer):
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
///   [`BoxOutOfOrder`](crate::MuxError::BoxOutOfOrder) or
///   [`DuplicateBox`](crate::MuxError::DuplicateBox), and a file
///   declared over without a `moov` is
///   [`MissingMandatoryBox`](crate::MuxError::MissingMandatoryBox).
/// * The movie comes before any fragment: a fragment opened or closed, or a
///   sample handed over, before it is
///   [`BoxOutOfOrder`](crate::MuxError::BoxOutOfOrder) of the `moof`. The
///   samples are checked against the movie as they are handed over, as
///   [`MovieFragmentWriter`] checks them, and a movie it refuses is refused
///   before any of it is laid down.
/// * The `ftyp` handed over is laid down as it stands, unless it lists a
///   brand under which the `default-base-is-moof` every `tfhd` states shall
///   not be used ([`FileTypeBox::forbids_default_base_is_moof`]):
///   that is [`UnsupportedBrand`](crate::MuxError::UnsupportedBrand), and
///   nothing of it is laid down. Where none was handed over, the muxer
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
///   [`Sample`](crate::MuxError::Sample).
/// * The bytes are taken from [`poll_output`](Self::poll_output), one
///   [`OutputBytes`] a call, owned by whoever takes them. The caller drains
///   before handing over more: bytes are held until they are taken, so
///   writing on without polling has the muxer hold the whole file.
/// * An `Err` leaves the muxer failed for good,
///   [`AlreadyFinished`](crate::MuxError::AlreadyFinished) aside:
///   every later call reports that same failure again. The bytes made before
///   it are still there to take.
/// * [`finish`](Self::finish) declares the file over. Bytes are still taken
///   after it, but anything handed over then, or a second
///   [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::MuxError::AlreadyFinished).
///
/// # Examples
///
/// ```
/// use isobmff::boxes::{SampleFlags, TrackExtendsBox};
/// use isobmff::sample::{Sample, SampleProperties};
/// use isobmff::FragmentedMuxer;
/// # use isobmff_test_support::fragmented_movie;
/// // A file handed no brands, only the movie its fragments continue
/// let mut muxer = FragmentedMuxer::new();
/// muxer.handle_movie(fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO)))?;
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
/// let mut file = Vec::new();
/// while let Some(written) = muxer.poll_output() {
///     file.extend_from_slice(&written);
/// }
///
/// // The file opens with the brands the muxer declares, and the media data holds the samples end to end
/// assert_eq!(&file[4..8], b"ftyp");
/// assert!(file.ends_with(b"SAMPDATA"));
/// # Ok::<(), isobmff::MuxError>(())
/// ```
#[derive(Debug)]
pub struct FragmentedMuxer {
    output: MuxOutput,
    structure: FragmentedStructure,
    samples: Option<MovieFragmentWriter>,
}

impl FragmentedMuxer {
    /// Creates a muxer waiting at the start of a fragmented movie file
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
    /// * [`UnsupportedBrand`](crate::MuxError::UnsupportedBrand): a
    ///   brand listed forbids the `default-base-is-moof` the muxer writes;
    ///   nothing of the box is laid down.
    /// * [`BoxOutOfOrder`](crate::MuxError::BoxOutOfOrder): a box
    ///   was handed over before them.
    /// * [`Box`](crate::MuxError::Box): the box does not write.
    /// * [`AlreadyFinished`](crate::MuxError::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the muxer keeps and reports
    ///   again for every call after it.
    pub fn handle_file_type(&mut self, file_type: FileTypeBox) -> Result<(), MuxError> {
        self.output.writing()?;
        let mut write_file_type = || -> Result<(), MuxError> {
            if file_type.forbids_default_base_is_moof() {
                return Err(MuxError::UnsupportedBrand);
            }
            self.write_value(&file_type)
        };
        let written = write_file_type();

        self.output.record(written)
    }

    /// Takes the movie the fragments continue, and lays it down
    ///
    /// The samples handed over from here on are checked against it.
    ///
    /// # Errors
    ///
    /// * [`Sample`](crate::MuxError::Sample): the movie is one
    ///   [`MovieFragmentWriter::new`] refuses — it carries no `mvex`, or its
    ///   sample tables lay samples out; nothing of it is laid down.
    /// * [`DuplicateBox`](crate::MuxError::DuplicateBox): a movie
    ///   continuing in fragments was handed over already.
    /// * [`Box`](crate::MuxError::Box): the box does not write.
    /// * [`AlreadyFinished`](crate::MuxError::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the muxer keeps and reports
    ///   again for every call after it.
    pub fn handle_movie(&mut self, movie: MovieBox) -> Result<(), MuxError> {
        self.output.writing()?;
        let mut write_movie = || -> Result<(), MuxError> {
            let samples = MovieFragmentWriter::new(&movie)?;
            if self.structure.is_at_start() {
                self.write_value(&default_file_type())?;
            }
            self.write_value(&movie)?;
            self.samples = Some(samples);

            Ok(())
        };
        let written = write_movie();

        self.output.record(written)
    }

    /// Opens a fragment, which the samples handed over next are laid out in
    ///
    /// `sequence_number` is what its `mfhd` states, which §8.8.5 has increase
    /// over the fragments of a presentation.
    ///
    /// # Errors
    ///
    /// * [`BoxOutOfOrder`](crate::MuxError::BoxOutOfOrder): the
    ///   movie was not handed over first.
    /// * [`Sample`](crate::MuxError::Sample): what the sample layer
    ///   makes of the call.
    /// * [`AlreadyFinished`](crate::MuxError::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the muxer keeps and reports
    ///   again for every call after it.
    pub fn begin_fragment(&mut self, sequence_number: u32) -> Result<(), MuxError> {
        self.output.writing()?;
        let begun = self.samples().and_then(|samples| {
            samples
                .begin_fragment(sequence_number)
                .map_err(MuxError::from)
        });

        self.output.record(begun)
    }

    /// Opens a fragment in which every track continues where the samples written for it reach, as [`MovieFragmentWriter::begin_fragment_continuing`] places them
    ///
    /// `sequence_number` is what its `mfhd` states, as for
    /// [`begin_fragment`](Self::begin_fragment).
    ///
    /// # Errors
    ///
    /// * [`BoxOutOfOrder`](crate::MuxError::BoxOutOfOrder): the
    ///   movie was not handed over first.
    /// * [`Sample`](crate::MuxError::Sample): what the sample layer
    ///   makes of the call.
    /// * [`AlreadyFinished`](crate::MuxError::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the muxer keeps and reports
    ///   again for every call after it.
    pub fn begin_fragment_continuing(&mut self, sequence_number: u32) -> Result<(), MuxError> {
        self.output.writing()?;
        let begun = self.samples().and_then(|samples| {
            samples
                .begin_fragment_continuing(sequence_number)
                .map_err(MuxError::from)
        });

        self.output.record(begun)
    }

    /// Takes a sample, and places it in the fragment that is open
    ///
    /// # Errors
    ///
    /// * [`BoxOutOfOrder`](crate::MuxError::BoxOutOfOrder): the
    ///   movie was not handed over first.
    /// * [`Sample`](crate::MuxError::Sample): what the sample layer
    ///   makes of the sample.
    /// * [`AlreadyFinished`](crate::MuxError::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the muxer keeps and reports
    ///   again for every call after it.
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), MuxError> {
        self.output.writing()?;
        let placed = self
            .samples()
            .and_then(|samples| samples.handle_sample(sample).map_err(MuxError::from));

        self.output.record(placed)
    }

    /// Closes the fragment that is open, and lays it down
    ///
    /// The `moof` and the `mdat` the sample layer made of it are written here,
    /// the media data moving into the file rather than being copied into it.
    ///
    /// # Errors
    ///
    /// * [`BoxOutOfOrder`](crate::MuxError::BoxOutOfOrder): the
    ///   movie was not handed over first.
    /// * [`Sample`](crate::MuxError::Sample): what the sample layer
    ///   makes of the fragment.
    /// * [`Box`](crate::MuxError::Box): the `moof` or the `mdat`
    ///   does not write.
    /// * [`AlreadyFinished`](crate::MuxError::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the muxer keeps and reports
    ///   again for every call after it.
    pub fn finish_fragment(&mut self) -> Result<(), MuxError> {
        self.output.writing()?;
        let mut write_fragment = || -> Result<(), MuxError> {
            self.samples()?.finish_fragment()?;
            while let Some((movie_fragment, media_data)) = self.samples()?.poll_fragment() {
                self.write_value(&movie_fragment)?;
                self.write_box(MediaDataBox::BOX_TYPE, media_data)?;
            }

            Ok(())
        };
        let written = write_fragment();

        self.output.record(written)
    }

    /// Hands over the bytes the file has been laid down as so far
    ///
    /// Reports `None` once they are used up: more samples are needed, or the
    /// file is over. Failure is reported by the calls that take the boxes and
    /// the samples, so this one never fails — a failed muxer hands over the
    /// bytes it had already made, then nothing from there on.
    pub fn poll_output(&mut self) -> Option<OutputBytes> {
        self.output.poll_output()
    }

    /// Declares the file over
    ///
    /// # Errors
    ///
    /// * [`Sample`](crate::MuxError::Sample): a fragment was left
    ///   open.
    /// * [`MissingMandatoryBox`](crate::MuxError::MissingMandatoryBox):
    ///   the movie was never handed over, so the file laid down is not a
    ///   fragmented movie file.
    /// * [`AlreadyFinished`](crate::MuxError::AlreadyFinished): the
    ///   file was already declared over.
    /// * The failure of a previous call, which the muxer keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<(), MuxError> {
        self.output.writing()?;
        let mut finish_file = || -> Result<(), MuxError> {
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
    fn samples(&mut self) -> Result<&mut MovieFragmentWriter, MuxError> {
        self.samples.as_mut().ok_or(MuxError::BoxOutOfOrder {
            box_type: MovieFragmentBox::BOX_TYPE,
        })
    }

    /// Lays `value` down as the whole box it forms
    fn write_value<Value: BoxEncode + BoxDefinition>(
        &mut self,
        value: &Value,
    ) -> Result<(), MuxError> {
        let payload = whole_payload(value)?;

        self.write_box(Value::BOX_TYPE, payload)
    }

    /// Lays one box down where the structure places it, through the framing of the file
    ///
    /// A box the structure passes over is refused as
    /// [`BoxOutOfOrder`](crate::MuxError::BoxOutOfOrder).
    fn write_box(&mut self, box_type: BoxType, payload: Vec<u8>) -> Result<(), MuxError> {
        let header = whole_box_header(box_type, payload.len() as u64)?;

        match self.structure.handle_box_type(box_type)? {
            FragmentedDisposition::FileType
            | FragmentedDisposition::Movie
            | FragmentedDisposition::MovieFragment
            | FragmentedDisposition::MediaData => {}
            FragmentedDisposition::SegmentIndex
            | FragmentedDisposition::MovieFragmentRandomAccess
            | FragmentedDisposition::Skip => {
                return Err(MuxError::BoxOutOfOrder { box_type });
            }
        }

        self.output.write_box(header, [payload])
    }
}

impl Default for FragmentedMuxer {
    fn default() -> Self {
        Self::new()
    }
}

/// Brands the muxer declares where none were handed over, those the widest layout it lays down requires
fn default_file_type() -> FileTypeBox {
    FileTypeBox::new(FourCc::new(*b"iso6"), 0, alloc::vec![FourCc::new(*b"iso6")])
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use isobmff_boxes::{FileTypeBox, MovieBox, MovieFragmentBox, SampleFlags, TrackExtendsBox};
    use isobmff_core::{BoxDecode, BoxDefinition, FourCc};
    use isobmff_test_support::{file_type, fragmented_movie, unfragmented_movie};

    use super::{FragmentedMuxer, MuxError, default_file_type};
    use crate::sample::{Sample, SampleProperties};

    /// Movie of one track continued in fragments, whose defaults a `trex` states
    fn movie() -> MovieBox {
        fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO))
    }

    /// A sample of the track the movie declares
    fn sample() -> Sample {
        Sample::new(
            SampleProperties {
                track_id: 1,
                decode_time: 0,
                sample_duration: 1_024,
                sample_composition_time_offset: 0,
                sample_flags: SampleFlags::ZERO,
                sample_description_index: 1,
            },
            b"SAMP".to_vec(),
        )
    }

    /// The bytes the muxer has laid down, drained to the end
    fn drained(muxer: &mut FragmentedMuxer) -> Vec<u8> {
        let mut file = Vec::new();
        while let Some(written) = muxer.poll_output() {
            file.extend_from_slice(&written);
        }

        file
    }

    #[test]
    fn a_file_handed_no_brands_opens_with_the_brands_the_muxer_declares() {
        let mut muxer = FragmentedMuxer::new();

        muxer.handle_movie(movie()).unwrap();
        muxer.finish().unwrap();
        let file = drained(&mut muxer);

        assert_eq!(
            FileTypeBox::decode(&file).map(|(file_type, rest)| (file_type, rest.get(4..8))),
            Ok((default_file_type(), Some(b"moov".as_slice())))
        );
    }

    #[test]
    fn the_brands_handed_over_are_laid_down_as_they_stand() {
        let mut muxer = FragmentedMuxer::new();

        muxer.handle_file_type(file_type()).unwrap();
        muxer.handle_movie(movie()).unwrap();
        muxer.finish().unwrap();
        let file = drained(&mut muxer);

        assert_eq!(
            FileTypeBox::decode(&file).map(|(file_type, rest)| (file_type, rest.get(4..8))),
            Ok((file_type(), Some(b"moov".as_slice())))
        );
    }

    #[test]
    fn brands_forbidding_default_base_is_moof_are_rejected_before_anything_is_laid_down() {
        let mut muxer = FragmentedMuxer::new();

        assert_eq!(
            muxer.handle_file_type(FileTypeBox::new(
                FourCc::new(*b"iso6"),
                0,
                alloc::vec![FourCc::new(*b"iso6"), FourCc::new(*b"isom")],
            )),
            Err(MuxError::UnsupportedBrand)
        );
        assert_eq!(muxer.poll_output(), None);
    }

    #[test]
    fn a_fragment_opened_or_a_sample_handed_over_before_the_movie_is_rejected() {
        let mut opened = FragmentedMuxer::new();
        let mut handed_a_sample = FragmentedMuxer::new();

        opened.handle_file_type(file_type()).unwrap();

        assert_eq!(
            opened.begin_fragment(1),
            Err(MuxError::BoxOutOfOrder {
                box_type: MovieFragmentBox::BOX_TYPE
            })
        );
        assert_eq!(
            handed_a_sample.handle_sample(sample()),
            Err(MuxError::BoxOutOfOrder {
                box_type: MovieFragmentBox::BOX_TYPE
            })
        );
    }

    #[test]
    fn a_movie_continued_in_no_fragments_is_rejected_before_anything_is_laid_down() {
        let mut muxer = FragmentedMuxer::new();

        assert!(matches!(
            muxer.handle_movie(unfragmented_movie()),
            Err(MuxError::Sample {
                error: crate::sample::Error::MissingMovieExtends
            })
        ));
        assert_eq!(muxer.poll_output(), None);
    }

    #[test]
    fn a_sample_the_movie_does_not_resolve_is_rejected_where_it_is_handed_over() {
        let refused = |sample: Sample| {
            let mut muxer = FragmentedMuxer::new();
            muxer.handle_movie(movie()).unwrap();
            muxer.begin_fragment(1).unwrap();

            muxer.handle_sample(sample)
        };

        assert!(matches!(
            refused(Sample::new(
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
        assert!(matches!(
            refused(Sample::new(
                SampleProperties {
                    track_id: 1,
                    decode_time: 0,
                    sample_duration: 1_024,
                    sample_composition_time_offset: 0,
                    sample_flags: SampleFlags::ZERO,
                    sample_description_index: 2
                },
                b"SAMP".to_vec()
            )),
            Err(MuxError::Sample {
                error: crate::sample::Error::UnknownSampleDescriptionIndex {
                    track_id: 1,
                    sample_description_index: 2,
                    ..
                }
            })
        ));
    }

    #[test]
    fn a_sample_handed_over_while_no_fragment_is_open_is_rejected() {
        let mut muxer = FragmentedMuxer::new();

        muxer.handle_movie(movie()).unwrap();

        assert!(matches!(
            muxer.handle_sample(sample()),
            Err(MuxError::Sample {
                error: crate::sample::Error::NoFragmentOpen
            })
        ));
    }

    #[test]
    fn a_file_declared_over_without_a_movie_is_rejected() {
        let mut muxer = FragmentedMuxer::new();

        muxer.handle_file_type(file_type()).unwrap();

        assert_eq!(
            muxer.finish(),
            Err(MuxError::MissingMandatoryBox {
                box_type: MovieBox::BOX_TYPE
            })
        );
    }

    #[test]
    fn anything_handed_over_after_finishing_is_rejected() {
        let mut muxer = FragmentedMuxer::new();

        muxer.handle_file_type(file_type()).unwrap();
        muxer.handle_movie(movie()).unwrap();
        muxer.finish().unwrap();

        assert_eq!(
            muxer.handle_sample(sample()),
            Err(MuxError::AlreadyFinished)
        );
        assert_eq!(muxer.finish(), Err(MuxError::AlreadyFinished));
    }
}
