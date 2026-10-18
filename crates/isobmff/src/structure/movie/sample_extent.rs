//! [`MovieSampleExtentFsm`], where the samples of a movie file lie, read as it arrives without gathering their bytes, fragmented or not

use alloc::collections::VecDeque;

use isobmff_boxes::{FileTypeBox, MovieBox, MovieFragmentBox};
use isobmff_core::BoxDefinition;

use crate::sample::{SampleExtent, TrackDecodeTimes, movie_fragment, sample_table};
use crate::sequence::BoxEvent;
use crate::structure::fragmented_movie::{FragmentedDisposition, FragmentedStructure};
use crate::structure::{DemuxInput, DemuxLimits, Error, WantedInput, WholeBoxReader};

/// Reads where the samples a movie file carries lie, fragmented or not, taking it in order as it arrives
///
/// A movie file declares its samples in the sample tables of its one movie
/// (ISO/IEC 14496-12 §8.2.1), and a fragmented one in each movie fragment as
/// well, against the movie it continues (Annex A.8, §8.8). This FSM reads the
/// file through the same layers [`MovieDemuxFsm`](crate::structure::MovieDemuxFsm) does
/// short of gathering the samples: the framing of the file into boxes, the
/// structure that says what each top-level box is, the reading of the `ftyp`,
/// the `moov` and each `moof` into values, and the resolution of their tables
/// into a [`SampleExtent`] for every sample — what is true of it and the
/// bytes of the file it spans. The media data, and every other box, is passed
/// over unread, so the file is read once, in order, whatever order its media
/// data lies in.
///
/// # Contract
///
/// * The file is handed over from its first byte, in order and cut anywhere,
///   each cut at the offset it was read at, where
///   [`wanted_input`](Self::wanted_input) names, and the extents it resolved
///   are taken from [`poll_sample_extent`](Self::poll_sample_extent). The
///   caller drains before handing over more: the extents of a `moov` or a
///   `moof` are held until they are taken. Every offset an extent names is a
///   file offset, counting from the first byte of the file as the boxes count
///   theirs (§8.7.5, §8.8.7).
/// * The `ftyp` and the `moov` are there to read once they have arrived:
///   [`file_type`](Self::file_type) and [`movie`](Self::movie).
/// * The extents of the `moov` come in the order their bytes lie in the
///   file, then those of each `moof` in the order it declares them.
/// * A `moof` before the `moov`, as in a media segment delivered apart from
///   its movie, is [`BoxOutOfOrder`](crate::structure::Error::BoxOutOfOrder).
/// * An `Err` leaves the FSM failed for good,
///   [`AlreadyFinished`](crate::structure::Error::AlreadyFinished) and
///   [`UnwantedInput`](crate::structure::Error::UnwantedInput) aside: every later call
///   reports that same failure again. The extents resolved before it are
///   still there to take, those of a `moov` or a `moof` whose tables are
///   refused part way among them.
/// * [`finish`](Self::finish) declares the file over, and reports what any
///   layer makes of the end of it: a box left open, the `moov` never come.
///   Extents are still taken after it, but anything handed over then, or a
///   second [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::structure::Error::AlreadyFinished).
///
/// # Examples
///
/// ```
/// use isobmff::boxes::{SampleFlags, TrackExtendsBox};
/// use isobmff::sample::{Sample, SampleProperties};
/// use isobmff::structure::{FragmentedMuxFsm, MovieSampleExtentFsm};
/// # use isobmff_test_support::{file_type, fragmented_movie};
/// // A file of one fragment carrying one sample of track 1
/// let mut mux_fsm = FragmentedMuxFsm::new();
/// mux_fsm.handle_file_type(file_type())?;
/// mux_fsm.handle_movie(fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO)))?;
/// mux_fsm.begin_fragment(1)?;
/// mux_fsm.handle_sample(Sample::new(
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
/// mux_fsm.finish_fragment()?;
/// mux_fsm.finish()?;
///
/// // The bytes the mux FSM wrote are the file
/// let mut file = Vec::new();
/// while let Some(written) = mux_fsm.poll_output() {
///     file.extend_from_slice(&written);
/// }
///
/// // The file is handed over in order, in whatever lengths it comes
/// let mut extent_fsm = MovieSampleExtentFsm::new();
/// for (offset, arriving) in (0..).step_by(7).zip(file.chunks(7)) {
///     extent_fsm.handle_input(offset, arriving)?;
/// }
/// extent_fsm.finish()?;
///
/// // The extent names the bytes of the sample in the file
/// let extent = extent_fsm.poll_sample_extent().unwrap();
/// let range = usize::try_from(extent.extent().start).unwrap()..usize::try_from(extent.extent().end).unwrap();
/// assert_eq!(&file[range], b"SAMP");
/// assert_eq!(extent_fsm.poll_sample_extent(), None);
/// # Ok::<(), isobmff::structure::Error>(())
/// ```
#[derive(Debug)]
pub struct MovieSampleExtentFsm {
    input: DemuxInput<()>,
    structure: FragmentedStructure,
    decode_times: TrackDecodeTimes,
    open: Option<Open>,
    file_type: Option<FileTypeBox>,
    movie: Option<MovieBox>,
    extents: VecDeque<SampleExtent>,
    limits: DemuxLimits,
}

/// The top-level box that started and is read whole until it ends
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
}

impl MovieSampleExtentFsm {
    /// Creates an FSM waiting at the start of a movie file, bounded by the limits [`DemuxLimits::new`] states
    #[must_use]
    pub const fn new() -> Self {
        Self::with_limits(DemuxLimits::new())
    }

    /// Creates an FSM waiting at the start of a movie file, bounded by `limits`
    ///
    /// Of `limits`, the payload a box read into a value may reach and the
    /// samples one `moov` or `moof` may declare apply; nothing is gathered,
    /// so the limits of [`DemuxLimits::sample_reader`] do not.
    #[must_use]
    pub const fn with_limits(limits: DemuxLimits) -> Self {
        Self {
            input: DemuxInput::new(()),
            structure: FragmentedStructure::new(),
            decode_times: TrackDecodeTimes::unknown(),
            open: None,
            file_type: None,
            movie: None,
            extents: VecDeque::new(),
            limits,
        }
    }

    /// Takes bytes of the file read at `offset`, and resolves the extents of the samples the boxes they complete declare
    ///
    /// `offset` is a file offset, counted from the first byte of the file as a
    /// base data offset is (ISO/IEC 14496-12 §8.8.7). Bytes at the offset the
    /// input taken in order stands at are taken whole as its continuation.
    /// Empty input is taken as nothing. What the input resolved is then taken
    /// from [`poll_sample_extent`](Self::poll_sample_extent).
    ///
    /// # Errors
    ///
    /// * [`BoxOutOfOrder`](crate::structure::Error::BoxOutOfOrder),
    ///   [`DuplicateBox`](crate::structure::Error::DuplicateBox): what the
    ///   structure makes of a top-level box arriving where it does.
    /// * [`PayloadLimitExceeded`](crate::structure::Error::PayloadLimitExceeded):
    ///   a box read into a value reaches past the limit the FSM gathers.
    /// * [`Sequence`](crate::structure::Error::Sequence): what the framing
    ///   of the file makes of the input.
    /// * [`Box`](crate::structure::Error::Box): a box read into a value
    ///   does not decode.
    /// * [`Sample`](crate::structure::Error::Sample): what the samples make of the
    ///   tables of the movie or of a fragment.
    /// * [`AlreadyFinished`](crate::structure::Error::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * [`UnwantedInput`](crate::structure::Error::UnwantedInput): `offset` is
    ///   not where the input taken in order stands. The FSM is not failed by
    ///   it.
    /// * The failure of a previous call, which the FSM keeps and reports again
    ///   for every call after it.
    pub fn handle_input(&mut self, offset: u64, input: &[u8]) -> Result<(), Error> {
        self.input.handle_input(offset, input)?;
        let read = self.read_framed();

        self.input.record(read)
    }

    /// Takes the next extent the file handed over so far resolved
    ///
    /// Reports `None` once they are used up: more of the file is needed.
    /// Failure is reported by the calls that take it, so this one never fails.
    pub fn poll_sample_extent(&mut self) -> Option<SampleExtent> {
        self.extents.pop_front()
    }

    /// Returns the one read wanted next, or `None` once the file is declared over or the FSM has failed
    ///
    /// The continuation of the input taken in order is wanted, at the offset
    /// it stands at and with no length.
    #[must_use]
    pub fn wanted_input(&self) -> Option<WantedInput> {
        self.input.wanted_input()
    }

    /// Returns the `ftyp`, once it has arrived
    #[must_use]
    pub const fn file_type(&self) -> Option<&FileTypeBox> {
        self.file_type.as_ref()
    }

    /// Returns the `moov`, once it has arrived
    #[must_use]
    pub const fn movie(&self) -> Option<&MovieBox> {
        self.movie.as_ref()
    }

    /// Declares the file over
    ///
    /// # Errors
    ///
    /// * [`Sequence`](crate::structure::Error::Sequence): the file ended
    ///   inside a box.
    /// * [`Box`](crate::structure::Error::Box): a box read into a value,
    ///   declaring no total, does not decode.
    /// * [`MissingMandatoryBox`](crate::structure::Error::MissingMandatoryBox):
    ///   the file carried no `moov`.
    /// * [`Sample`](crate::structure::Error::Sample): what the samples make of the
    ///   tables of the box the file ended with.
    /// * [`AlreadyFinished`](crate::structure::Error::AlreadyFinished): the
    ///   file was already declared over.
    /// * The failure of a previous call.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.input.finish_framing()?;
        let read = self.read_framed();
        self.input.record(read)?;
        let checked = self.structure.finish();

        self.input.finish(checked)
    }

    /// Reads every event the framing made, reading the `ftyp`, the `moov` and each `moof` whole and resolving the extents they declare
    fn read_framed(&mut self) -> Result<(), Error> {
        while let Some((start, event)) = self.input.poll_event() {
            match event {
                BoxEvent::Header(header) => self
                    .structure
                    .handle_box_type(header.box_type())
                    .and_then(|disposition| {
                        self.open = match disposition {
                            FragmentedDisposition::FileType => Some(Open::FileType(
                                WholeBoxReader::begin(header, self.limits.payload())?,
                            )),
                            FragmentedDisposition::Movie => Some(Open::Movie(
                                WholeBoxReader::begin(header, self.limits.payload())?,
                            )),
                            FragmentedDisposition::MovieFragment => Some(Open::MovieFragment {
                                reader: WholeBoxReader::begin(header, self.limits.payload())?,
                                moof_start: start,
                            }),
                            FragmentedDisposition::SegmentIndex
                            | FragmentedDisposition::MovieFragmentRandomAccess
                            | FragmentedDisposition::MediaData
                            | FragmentedDisposition::Skip => None,
                        };

                        Ok(())
                    }),
                BoxEvent::Payload(payload) => match &mut self.open {
                    Some(Open::FileType(reader)) => reader.handle_payload(payload),
                    Some(Open::Movie(reader)) => reader.handle_payload(payload),
                    Some(Open::MovieFragment { reader, .. }) => reader.handle_payload(payload),
                    None => Ok(()),
                },
                BoxEvent::End => match self.open.take() {
                    Some(Open::FileType(reader)) => reader
                        .finish()
                        .map(|file_type| self.file_type = Some(file_type)),
                    Some(Open::Movie(reader)) => reader.finish().and_then(|movie| {
                        for extent in
                            sample_table::sample_extents(&movie, self.limits.resolved_samples())
                        {
                            self.extents.push_back(extent?);
                        }
                        self.decode_times = TrackDecodeTimes::new(&movie)?;
                        self.movie = Some(movie);

                        Ok(())
                    }),
                    Some(Open::MovieFragment { reader, moof_start }) => {
                        reader.finish().and_then(|movie_fragment| {
                            // Why not unreachable: the structure placed the `moof`
                            // after the `moov`, so the movie is there, and the
                            // fallback repeats what the structure answers a `moof`
                            // before it with, in place of a panic the lints forbid.
                            let Some(movie) = self.movie.as_ref() else {
                                return Err(Error::BoxOutOfOrder {
                                    box_type: MovieFragmentBox::BOX_TYPE,
                                });
                            };

                            for extent in movie_fragment::sample_extents(
                                &movie_fragment,
                                movie,
                                moof_start,
                                &mut self.decode_times,
                                self.limits.resolved_samples(),
                            )? {
                                self.extents.push_back(extent?);
                            }

                            Ok(())
                        })
                    }
                    None => Ok(()),
                },
            }?;
        }

        Ok(())
    }
}

impl Default for MovieSampleExtentFsm {
    fn default() -> Self {
        Self::new()
    }
}
