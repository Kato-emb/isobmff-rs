//! [`Muxer`], which writes samples into a movie file or a segment

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use isobmff::file_level::supports_default_base_is_moof;
use isobmff::framing::{BoxWriter, fill};
use isobmff::media_data::MediaDataWriter;
use isobmff::structure_data::{
    FragmentedOptions, MovieFragmentWriter, SampleTableWriter, SegmentOptions,
};
use isobmff::{ByteRange, Output, Sample, WriteError};
use isobmff_boxes::{FileTypeBox, MovieBox, SegmentTypeBox};
use isobmff_core::BoxDefinition;

use crate::{FileStructure, MuxLimits, NonFragmentedOptions};

mod error;
mod recoverable;

pub use self::error::MuxError;
use self::recoverable::Recoverable;

/// Writer of a movie file or a segment from the samples a caller hands it
///
/// The muxer does no I/O. It takes the brands, the movie and the samples
/// of the file, and returns the file a box at file level at a time from
/// [`poll_output`](Self::poll_output), each an [`Output`] with the file
/// offset it goes to: they come in the order they lie in the file, each at
/// the end of those before it, so writing them one after another gives the
/// file, as a caller that cannot seek does. How the samples are laid
/// out is the [`FileStructure`] the muxer is made for, with the options of
/// that structure. The muxer is the
/// writers of the layers composed: a caller that lays a file out otherwise
/// writes it with them, as the [`media_data`](isobmff::media_data) example
/// does.
///
/// The movie is the caller's `moov`, its tracks and their sample entries
/// already in place, such as the movie a [`Demuxer`](crate::Demuxer) read.
/// The muxer declares the samples in it, or in movie fragments, in tables
/// of its own, and refuses samples it would not declare in the file it
/// writes: those of a track, a sample entry or a data reference entry in the
/// file itself the movie does not declare.
///
/// A refused call leaves the muxer as it was, but for a call to
/// [`finish`](Self::finish) that fails once the muxer holds a movie, which
/// ends the file.
///
/// # Examples
///
/// ```
/// use isobmff::boxes::{DataEntry, DataEntryUrlBox, MovieBox, SampleDescription, SampleEntry, TrackBox};
/// use isobmff::{FourCc, Output, Sample, SampleProperties};
/// use isobmff_mux::{DemuxEvent, Demuxer, Muxer, NonFragmentedOptions};
///
/// // A movie of one track, its samples described by one sample entry, and in the file itself
/// let mut trak = TrackBox::default();
/// trak.tkhd.track_id = 1;
/// trak.mdia.mdhd.timescale = 1000;
/// let mut entry = SampleEntry::default();
/// entry.format = FourCc::new(*b"mp4a");
/// entry.data_reference_index = 1;
/// trak.mdia.minf.stbl.stsd.sample_descriptions = vec![SampleDescription::Typed(entry)];
/// trak.mdia.minf.dinf.dref.data_entries = vec![DataEntry::Url(DataEntryUrlBox::default())];
/// let mut movie = MovieBox::default();
/// movie.mvhd.timescale = 1000;
/// movie.mvhd.next_track_id = 2;
/// movie.trak = vec![trak];
///
/// // Two samples written together, then the `moov` at the end
/// let mut muxer = Muxer::non_fragmented(NonFragmentedOptions::default());
/// muxer.handle_movie(movie)?;
/// let sample = |decoding_time, data: &[u8]| {
///     let properties = SampleProperties {
///         track_id: 1,
///         decoding_time,
///         sample_duration: 1024,
///         sample_composition_time_offset: 0,
///         sample_flags: Default::default(),
///         sample_description_index: 1,
///     };
///     Sample::new(properties, data.to_vec())
/// };
/// muxer.handle_samples([sample(0, b"AB"), sample(1024, b"CD")])?;
/// muxer.finish()?;
/// let file: Vec<u8> = core::iter::from_fn(|| muxer.poll_output()).flat_map(Output::into_bytes).collect();
///
/// // A demuxer reads the same samples back
/// let mut demuxer = Demuxer::new();
/// let mut samples = Vec::new();
/// while let Some(wanted) = demuxer.wanted_input() {
///     let input = file.get(usize::try_from(wanted.offset())?..).unwrap_or_default();
///     let event = match input {
///         [] => demuxer.finish(),
///         input => demuxer.handle_input(wanted.offset(), input),
///     };
///     samples.extend(core::iter::from_fn(|| demuxer.poll_sample()));
///     event?;
/// }
/// assert_eq!(samples, [sample(0, b"AB"), sample(1024, b"CD")]);
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct Muxer {
    structure: Structure,
    boxes: BoxWriter,
    structure_data: Option<StructureDataWriter>,
    media_data: MediaDataWriter,
    /// Boxes written and not yet polled, in the order they lie in the file
    output: VecDeque<Output>,
    /// Space the `free` box reserves for the `sidx` of an indexed segment, once it is written
    reserved_segment_index: Option<ByteRange>,
    /// Whether the `ftyp`, or the `styp` of a segment, is written
    file_type: bool,
    finished: bool,
    limits: MuxLimits,
    /// What the muxer keeps of a recoverable file, once it holds the movie
    recoverable: Option<Recoverable>,
}

/// Structure of the file a muxer writes, and the options it writes it with
#[derive(Clone, Copy, Debug)]
enum Structure {
    NonFragmented(NonFragmentedOptions),
    Fragmented(FragmentedOptions),
    Segment(SegmentOptions),
}

impl Muxer {
    /// Creates the muxer of a movie file that is not fragmented, written with `options`
    #[must_use]
    pub fn non_fragmented(options: NonFragmentedOptions) -> Self {
        Self::new(Structure::NonFragmented(options))
    }

    /// Creates the muxer of a fragmented movie file, written with `options`
    ///
    /// # Examples
    ///
    /// ```
    /// use isobmff::structure_data::FragmentedOptions;
    /// use isobmff_mux::{FileStructure, Muxer};
    ///
    /// // Fragments numbered from 1, the file ending with an `mfra`
    /// let mut options = FragmentedOptions::default();
    /// options.movie_fragment_random_access = true;
    /// let muxer = Muxer::fragmented(options);
    /// assert_eq!(muxer.file_structure(), FileStructure::Fragmented);
    /// ```
    #[must_use]
    pub fn fragmented(options: FragmentedOptions) -> Self {
        Self::new(Structure::Fragmented(options))
    }

    /// Creates the muxer of a media segment, written with `options`
    ///
    /// Where a `sidx` indexes the segment, the subsegments are the calls of
    /// [`handle_samples`](Self::handle_samples): the muxer reserves the space
    /// of the `sidx` with a `free` box before the first `moof`, and
    /// [`finish`](Self::finish) writes the `sidx` over it, so that the
    /// segment is never held in memory. A call past
    /// [`SegmentOptions::segment_index`] is refused, and fewer calls leave the
    /// `free` box, a segment without a `sidx`.
    ///
    /// # Examples
    ///
    /// ```
    /// use isobmff::structure_data::SegmentOptions;
    /// use isobmff_mux::{FileStructure, Muxer};
    ///
    /// // Fragments numbered from 1, the segment of 4 subsegments indexed by a `sidx`
    /// let mut options = SegmentOptions::default();
    /// options.segment_index = Some(4);
    /// let muxer = Muxer::segment(options);
    /// assert_eq!(muxer.file_structure(), FileStructure::Segment);
    /// ```
    #[must_use]
    pub fn segment(options: SegmentOptions) -> Self {
        Self::new(Structure::Segment(options))
    }

    /// Replaces the limits the muxer writes the file up to, which are the defaults of [`MuxLimits::new`] until then
    #[must_use]
    pub const fn with_limits(mut self, limits: MuxLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Returns the structure of the file the muxer writes
    ///
    /// A recoverable file is one that is not fragmented, but one
    /// [`finish`](Self::finish) did not lay out again is the fragmented movie
    /// file written.
    #[must_use]
    pub fn file_structure(&self) -> FileStructure {
        match self.structure {
            Structure::NonFragmented(_)
                if self.finished
                    && self
                        .recoverable
                        .as_ref()
                        .is_some_and(|recoverable| !recoverable.laid_out()) =>
            {
                FileStructure::Fragmented
            }
            Structure::NonFragmented(_) => FileStructure::NonFragmented,
            Structure::Fragmented(_) => FileStructure::Fragmented,
            Structure::Segment(_) => FileStructure::Segment,
        }
    }

    /// Creates the muxer of a file of `structure`
    fn new(structure: Structure) -> Self {
        Self {
            structure,
            boxes: BoxWriter::new(),
            structure_data: None,
            media_data: MediaDataWriter::new(),
            output: VecDeque::new(),
            reserved_segment_index: None,
            file_type: false,
            finished: false,
            limits: MuxLimits::new(),
            recoverable: None,
        }
    }

    /// Writes the brands of the file: its `ftyp`, or the `styp` of a segment, of the same brands (§4.3, §8.14.2)
    ///
    /// A file of movie fragments, a recoverable one included, or a segment,
    /// takes the first byte of each `moof` as the base data offset of its
    /// track fragments, which a reader supports under `iso5` and later brands
    /// (§8.8.7.1): its major brand or a compatible brand is one of `iso5` to
    /// `isod`, as [`supports_default_base_is_moof`](isobmff::supports_default_base_is_moof)
    /// says.
    ///
    /// # Errors
    ///
    /// * [`DuplicateBox`](MuxError::DuplicateBox): the brands are written
    ///   already.
    /// * [`BoxOutOfOrder`](MuxError::BoxOutOfOrder): a box is written
    ///   already, which the brands go before.
    /// * [`UnsupportedBrand`](MuxError::UnsupportedBrand): the file is
    ///   fragmented, recoverable or a segment, and its brands name none of
    ///   `iso5` to `isod`.
    /// * [`AlreadyFinished`](WriteError::AlreadyFinished): the file is finished.
    pub fn handle_file_type(&mut self, file_type: FileTypeBox) -> Result<(), MuxError> {
        let box_type = match self.structure {
            Structure::Segment(_) => SegmentTypeBox::BOX_TYPE,
            Structure::Fragmented(_) | Structure::NonFragmented(_) => FileTypeBox::BOX_TYPE,
        };
        self.refuse_if_finished()?;
        if self.file_type {
            return Err(MuxError::DuplicateBox { box_type });
        }
        if self.boxes.next_offset() > 0 {
            return Err(MuxError::BoxOutOfOrder { box_type });
        }
        let bytes = match self.structure {
            Structure::NonFragmented(options) if !options.recoverable => {
                self.boxes.write_box(&file_type)?
            }
            Structure::NonFragmented(_) | Structure::Fragmented(_) => {
                refuse_brands_before_iso5(&file_type)?;
                self.boxes.write_box(&file_type)?
            }
            Structure::Segment(_) => {
                refuse_brands_before_iso5(&file_type)?;
                let mut styp = SegmentTypeBox::default();
                styp.major_brand = file_type.major_brand;
                styp.minor_version = file_type.minor_version;
                styp.compatible_brands = file_type.compatible_brands;
                self.boxes.write_box(&styp)?
            }
        };
        self.output.push_back(bytes);
        self.file_type = true;

        Ok(())
    }

    /// Takes the movie whose tracks the samples are of, and writes its `moov` where the file has it
    ///
    /// A fragmented file writes the `moov` here, a file that is not
    /// fragmented at its end, once [`finish`](Self::finish) has its sample
    /// tables, and a segment writes none: its movie is that of its
    /// initialization segment, which the samples are checked against. A
    /// recoverable file writes the 16-byte `free` the header of its `mdat`
    /// takes the place of, and the `moov` of its movie fragments, here, and
    /// the `moov` of its sample tables at its end.
    ///
    /// The muxer writes how the movie declares its samples: the movie gives
    /// the tracks, their sample entries, data references and edit lists.
    /// The tables of each sample table that state what the samples state
    /// (`stts`, `ctts`, `stsc`, `stsz` or `stz2`, `stco` or `co64`, `stss`,
    /// `sdtp`, `padb`, `stdp`; ISO/IEC 14496-12 §8.6, §8.7) are outputs of
    /// the muxer, and their values in `movie` are not used: a file that is
    /// not fragmented has them filled from its samples, and a fragmented file
    /// or a segment has them declare no sample. So is the `mvex` (§8.8.1): a
    /// file that is not fragmented has none, and a fragmented file or a
    /// segment has the `mvex` of `movie`, with a `trex` whose samples sample
    /// entry 1 describes added for each track it has none for. So are the
    /// durations of the `mvhd`, of each `tkhd` and of each `mdhd`, with the
    /// versions that hold them (§8.2.2, §8.3.2, §8.4.2), in a file that is
    /// not fragmented: [`finish`](Self::finish) replaces them with the
    /// duration of the samples of each track, or the sum of the edits of a
    /// track with an edit list that does not repeat, and the longest of
    /// those. A fragmented file or a segment writes them as `movie` gives
    /// them. A last edit of duration 0 of a fragmented `movie`, which lasts
    /// to the end of its media (§8.6.6.1), lasts to the end of the samples
    /// in a file that is not fragmented. In a
    /// file that is not fragmented, whose sample tables count decoding times
    /// from 0 (§8.6.1.2), a track whose first sample is at a later decoding
    /// time has the `media_time` of its edits moved back by as much. A
    /// movie read by a [`Demuxer`](crate::Demuxer) is taken as it is, and
    /// [`movie`](Self::movie) returns the movie in the form the muxer uses.
    ///
    /// # Errors
    ///
    /// * [`DuplicateBox`](MuxError::DuplicateBox): the muxer holds a movie
    ///   already.
    /// * [`SampleTableNotEmpty`](WriteError::SampleTableNotEmpty): a sample
    ///   group, sub-sample, sample auxiliary information or shadow sync table
    ///   of a track declares samples, which the muxer does not write from the
    ///   samples it takes.
    /// * [`Timeline`](WriteError::Timeline): the segment is indexed, and the
    ///   edit list of the first track of the movie does not map its
    ///   composition timeline.
    /// * [`Box`](WriteError::Box): the `moov` does not write.
    /// * [`AlreadyFinished`](WriteError::AlreadyFinished): the file is finished.
    pub fn handle_movie(&mut self, movie: MovieBox) -> Result<(), MuxError> {
        self.refuse_if_finished()?;
        if self.structure_data.is_some() {
            return Err(MuxError::DuplicateBox {
                box_type: MovieBox::BOX_TYPE,
            });
        }
        let structure_data = match self.structure {
            Structure::Fragmented(options) => {
                let writer = MovieFragmentWriter::fragmented(movie, options)?;
                let bytes = self.boxes.write_box(writer.movie())?;
                self.output.push_back(bytes);
                StructureDataWriter::Fragmented(writer)
            }
            Structure::Segment(options) => {
                StructureDataWriter::Fragmented(MovieFragmentWriter::segment(movie, options)?)
            }
            Structure::NonFragmented(options) if options.recoverable => {
                let writer = MovieFragmentWriter::fragmented(movie, FragmentedOptions::default())?;
                let (recoverable, free) = Recoverable::new(
                    &mut self.boxes,
                    writer.movie(),
                    self.limits.held_location_count(),
                )?;
                let bytes = self.boxes.write_box(writer.movie())?;
                self.output.extend([free, bytes]);
                self.recoverable = Some(recoverable);
                StructureDataWriter::Fragmented(writer)
            }
            Structure::NonFragmented(_) => {
                StructureDataWriter::NonFragmented(SampleTableWriter::new(movie)?)
            }
        };
        self.structure_data = Some(structure_data);

        Ok(())
    }

    /// Writes `samples` into one `mdat`, in their order, and declares them in a `moof` before it or in the sample tables of the `moov`
    ///
    /// The samples of each track go in decoding order, and those of one track
    /// are described by one sample entry. Their decoding times follow from
    /// the durations before them: across every call in a file that is not
    /// fragmented, within the call in one that is, whose movie fragments
    /// state the decoding time of each track (§8.8.12). No sample writes
    /// nothing.
    ///
    /// # Errors
    ///
    /// * [`MissingMovie`](MuxError::MissingMovie): the muxer holds no movie.
    /// * [`UnknownTrackId`](WriteError::UnknownTrackId),
    ///   [`UnknownSampleDescriptionIndex`](WriteError::UnknownSampleDescriptionIndex),
    ///   [`UnknownDataReferenceIndex`](WriteError::UnknownDataReferenceIndex),
    ///   [`ExternalDataReference`](WriteError::ExternalDataReference),
    ///   [`UnsupportedDataReference`](WriteError::UnsupportedDataReference): a
    ///   sample is of a track, a sample entry or a data reference entry in the
    ///   file itself the movie has none of.
    /// * [`MixedSampleDescriptions`](WriteError::MixedSampleDescriptions),
    ///   [`DecodingTimeMismatch`](WriteError::DecodingTimeMismatch): the
    ///   samples of a track are described by more than one sample entry, or
    ///   their decoding times do not follow from their durations. In a file
    ///   that is not fragmented, the first sample of a track without an edit
    ///   list is at decoding time 0, and its decoding time is the origin the
    ///   sample tables move the edits of one with an edit list back from.
    /// * [`MediaTimeOutOfRange`](WriteError::MediaTimeOutOfRange): in a file
    ///   that is not fragmented, an edit of a track starts before the first
    ///   sample of the track, which the moved edit could not reach.
    /// * [`SampleSizeOutOfRange`](WriteError::SampleSizeOutOfRange),
    ///   [`DataOffsetOutOfRange`](WriteError::DataOffsetOutOfRange): the media
    ///   data of a sample is longer, or lies further, than the structure-data
    ///   reaches.
    /// * [`SegmentIndexOutOfRange`](WriteError::SegmentIndexOutOfRange): the
    ///   segment is indexed, and its `sidx` cannot state the `moof` and the
    ///   `mdat` of the samples as a subsegment.
    /// * [`SegmentIndexCountMismatch`](WriteError::SegmentIndexCountMismatch):
    ///   the segment is indexed, and the call is past the number of
    ///   subsegments [`SegmentOptions::segment_index`] gives; nothing is
    ///   written.
    /// * [`AlreadyFinished`](WriteError::AlreadyFinished): the file is finished.
    pub fn handle_samples(
        &mut self,
        samples: impl IntoIterator<Item = Sample>,
    ) -> Result<(), MuxError> {
        self.refuse_if_finished()?;
        let structure_data = self.structure_data.as_mut().ok_or(MuxError::MissingMovie)?;
        let samples: Vec<Sample> = samples.into_iter().collect();
        if samples.is_empty() {
            return Ok(());
        }
        let header = self.media_data.header(&samples)?;
        let mut written = Vec::new();
        match structure_data {
            StructureDataWriter::NonFragmented(writer) => {
                // Why not self.boxes: the sample table writer checks the samples once the mdat has placed them, and a refused call leaves the muxer as it was
                let mut boxes = self.boxes.clone();
                let mdat_offset = boxes.next_offset();
                let (mdat, locations) =
                    self.media_data
                        .write(writer.movie(), &mut boxes, header, &samples)?;
                writer.handle_locations(&locations)?;
                self.boxes = boxes;
                written.push(Output::new(mdat_offset, mdat));
            }
            StructureDataWriter::Fragmented(writer) => {
                let reserving = if self.reserved_segment_index.is_none() {
                    writer.segment_index_len()
                } else {
                    None
                };
                let moof_offset = self
                    .boxes
                    .next_offset()
                    .saturating_add(reserving.unwrap_or_default());
                let (moof, locations) =
                    writer.movie_fragment(&samples, moof_offset, header.encoded_len())?;
                if let Some(total_bytes) = reserving {
                    let free = self.boxes.write_reserved(total_bytes)?;
                    self.reserved_segment_index =
                        Some(ByteRange::new(free.offset(), Some(total_bytes)));
                    written.push(free);
                }
                written.push(self.boxes.write_box(&moof)?);
                let mdat_offset = self.boxes.next_offset();
                let (mdat, _) =
                    self.media_data
                        .write(writer.movie(), &mut self.boxes, header, &samples)?;
                written.push(Output::new(mdat_offset, mdat));
                if let Some(recoverable) = &mut self.recoverable {
                    recoverable.handle_locations(&locations);
                }
            }
        }
        self.output.extend(written);

        Ok(())
    }

    /// Returns the next box at file level written, and the file offset it goes to, or `None` when the muxer has written none it has not returned
    ///
    /// The boxes come in the order they lie in the file, each at the end of
    /// the boxes before it, but for the last of a recoverable file and the
    /// `sidx` of an indexed segment, which [`finish`](Self::finish) writes
    /// over space reserved before: a sink that cannot seek stops there.
    pub fn poll_output(&mut self) -> Option<Output> {
        self.output.pop_front()
    }

    /// Ends the file: writes the `moov` of a file that is not fragmented, with the sample tables of its samples, the `mfra` of a fragmented file that lists its sync samples, or the `sidx` of an indexed segment over the space reserved for it
    ///
    /// The muxer then takes no more input, even where this call fails once
    /// the muxer holds a movie. The boxes written are still returned by
    /// [`poll_output`](Self::poll_output).
    ///
    /// The `sidx` of an indexed segment goes over the `free` box the muxer
    /// wrote before the first `moof`, just as long, as an output for bytes
    /// written already. A segment of fewer subsegments than
    /// [`SegmentOptions::segment_index`] gives keeps the `free` box: it is a
    /// segment without a `sidx`, whole. A segment of no samples, which
    /// reserves nothing, takes its `sidx` of no subsegment at its end.
    ///
    /// A recoverable file is laid out again: the `moov` of its sample tables
    /// comes at the end, then the output that writes the header of an `mdat`
    /// over the 16-byte `free` after the `ftyp`, the only output a muxer
    /// returns for bytes written already. A sink that writes the `moov` but
    /// not the header leaves a fragmented movie file whole. Where this call
    /// fails for a recoverable file, the muxer writes nothing more: the file
    /// is the fragmented movie file written, no byte of it missing, and
    /// [`file_structure`](Self::file_structure) returns
    /// [`Fragmented`](FileStructure::Fragmented).
    ///
    /// # Errors
    ///
    /// * [`MissingMovie`](MuxError::MissingMovie): the muxer holds no movie.
    /// * [`NotFinalised`](MuxError::NotFinalised): the file is recoverable,
    ///   and holds more samples than
    ///   [`MuxLimits::held_location_count`](crate::MuxLimits::held_location_count);
    ///   the file written is a fragmented movie file whole.
    /// * The failures of
    ///   [`SampleTableWriter::handle_locations`](isobmff::structure_data::SampleTableWriter::handle_locations),
    ///   such as [`DecodingTimeMismatch`](WriteError::DecodingTimeMismatch):
    ///   the file is recoverable, and the sample tables refused samples of
    ///   it, which the movie fragments declare, as a file that is not
    ///   fragmented does not let them follow on; the file written is a
    ///   fragmented movie file whole.
    /// * [`SegmentIndexCountMismatch`](WriteError::SegmentIndexCountMismatch):
    ///   the segment is indexed, and holds fewer subsegments than
    ///   [`SegmentOptions::segment_index`] gives; the segment keeps the
    ///   `free` box reserved for the `sidx`.
    /// * [`Box`](WriteError::Box): the `moov` or the `mfra` does not write.
    /// * [`AlreadyFinished`](WriteError::AlreadyFinished): the file is
    ///   finished, or a call to finish failed.
    pub fn finish(&mut self) -> Result<(), MuxError> {
        self.refuse_if_finished()?;
        let structure_data = self.structure_data.as_mut().ok_or(MuxError::MissingMovie)?;
        // Why not at the end: the writers take their sample tables, mfra and
        // sidx as they finish, so a second call after a failure here would
        // write the file without them
        self.finished = true;
        let bytes = match structure_data {
            StructureDataWriter::NonFragmented(writer) => {
                Some(self.boxes.write_box(writer.finish())?)
            }
            StructureDataWriter::Fragmented(writer) => {
                let (mfra, sidx) = writer.finish()?;
                if let Some(recoverable) = &mut self.recoverable {
                    let laid_out = recoverable.finish(&mut self.boxes);
                    self.boxes.finish()?;
                    self.output.extend(laid_out?);
                    return Ok(());
                }
                if let Some(sidx) = sidx {
                    match self.reserved_segment_index {
                        Some(reserved) => {
                            // Why not self.boxes: it counts file offsets in the order boxes are written, and the sidx goes over the free it has counted already
                            let sidx = BoxWriter::new().write_box(&sidx)?.into_bytes();
                            self.output.extend(fill(reserved, sidx)?);
                        }
                        None => self.output.push_back(self.boxes.write_box(&sidx)?),
                    }
                }
                mfra.map(|mfra| self.boxes.write_box(&mfra)).transpose()?
            }
        };
        self.output.extend(bytes);
        self.boxes.finish()?;

        Ok(())
    }

    /// Returns the movie in the form the muxer takes it in, as [`handle_movie`](Self::handle_movie) sets out
    ///
    /// It is `None` until [`handle_movie`](Self::handle_movie) takes a movie.
    /// For a fragmented file it is the `moov` the muxer wrote, and for a
    /// segment the `moov` of its initialization segment, which the muxer does
    /// not write. For a file that is not fragmented it is the `moov` that
    /// [`finish`](Self::finish) writes: its sample tables and durations are
    /// those of the samples once [`finish`](Self::finish) is called, and
    /// before it the tables declare no sample. For a recoverable file it is
    /// the `moov` of its movie fragments, and once the file is laid out again
    /// the `moov` of its sample tables.
    #[must_use]
    pub const fn movie(&self) -> Option<&MovieBox> {
        if let Some(recoverable) = &self.recoverable
            && let Some(movie) = recoverable.movie()
        {
            return Some(movie);
        }
        match &self.structure_data {
            Some(writer) => Some(writer.movie()),
            None => None,
        }
    }

    /// Refuses a call once the file is finished
    const fn refuse_if_finished(&self) -> Result<(), MuxError> {
        if self.finished {
            return Err(MuxError::Write {
                error: WriteError::AlreadyFinished,
            });
        }
        Ok(())
    }
}

/// Writer of the structure-data of a file: a movie fragment for each run of samples, or the sample tables of its movie
///
/// Each writer holds the movie it was made from, and refuses samples the
/// movie does not declare a track, a sample entry or a data reference entry
/// in the file itself for: the media data it writes is in the file itself.
#[derive(Debug)]
enum StructureDataWriter {
    /// Movie fragments, §8.8
    Fragmented(MovieFragmentWriter),
    /// The sample tables of the movie, §8.5
    NonFragmented(SampleTableWriter),
}

impl StructureDataWriter {
    /// Returns the movie the writer was made from, in the form it writes it
    const fn movie(&self) -> &MovieBox {
        match self {
            Self::Fragmented(writer) => writer.movie(),
            Self::NonFragmented(writer) => writer.movie(),
        }
    }
}

/// Refuses `file_type` where it names none of the brands [`supports_default_base_is_moof`] looks for
fn refuse_brands_before_iso5(file_type: &FileTypeBox) -> Result<(), MuxError> {
    if supports_default_base_is_moof(file_type) {
        return Ok(());
    }
    Err(MuxError::UnsupportedBrand {
        brand: file_type.major_brand,
    })
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use core::iter;

    use isobmff::structure_data::{
        FragmentedOptions, SampleTableWriter, SegmentOptions, StructureDataLimits,
        StructureDataReader, movie_fragment_random_access_start,
    };
    use isobmff::{
        Leniency, Output, Sample, SampleLocation, SampleProperties, SampleTableDisagreement,
        WriteError,
    };
    use isobmff_boxes::{
        ChunkOffsetBox, ChunkOffsetVariant, FileTypeBox, MovieBox, MovieExtendsBox, SampleFlags,
        SampleSizeBox, SampleSizeVariant, SampleSizes, SampleToChunkBox, SampleToChunkEntry,
        TimeToSampleBox, TimeToSampleEntry, TrackExtendsBox,
    };
    use isobmff_core::{BoxDecode, BoxDefinition, BoxType, Boxes, FourCc};
    use isobmff_test_support::fixtures::{encoded, movie, track, written};

    use super::{MuxError, Muxer};
    use crate::convert::to_u64;
    use crate::demuxer::Demuxer;
    use crate::file_structure::{FileStructure, NonFragmentedOptions};
    use crate::limits::MuxLimits;

    /// Returns a movie of track 1 of no sample, its tables of no entry, the movie in a timescale of 1000 and the track of 100
    fn one_track() -> MovieBox {
        let mut trak = track(1, 0, &[], &[], &[]);
        trak.mdia.mdhd.timescale = 100;
        trak.mdia.minf.stbl.stts = Some(TimeToSampleBox::default());
        let mut movie = movie([trak]);
        movie.mvhd.timescale = 1000;
        movie.mvhd.next_track_id = 2;
        movie
    }

    /// Returns [`one_track`] with the `trex` of its track, its samples described by sample entry 1
    fn one_fragmented_track() -> MovieBox {
        let mut movie = one_track();
        let mut trex = TrackExtendsBox::default();
        trex.track_id = 1;
        trex.default_sample_description_index = 1;
        let mut mvex = MovieExtendsBox::default();
        mvex.trex = Vec::from([trex]);
        movie.mvex = Some(mvex);
        movie
    }

    /// Returns the `ftyp` of `major_brand` and `compatible_brands`
    fn file_type(major_brand: &[u8; 4], compatible_brands: &[&[u8; 4]]) -> FileTypeBox {
        let mut ftyp = FileTypeBox::default();
        ftyp.major_brand = FourCc::new(*major_brand);
        ftyp.compatible_brands = compatible_brands
            .iter()
            .map(|&&brand| FourCc::new(brand))
            .collect();
        ftyp
    }

    /// Returns two samples of track 1 from decoding time 0, lasting 10 each, of the media data `AB` and `CDE`
    fn two_samples() -> [Sample; 2] {
        let sample = |decoding_time, data: &[u8]| {
            Sample::new(
                SampleProperties {
                    track_id: 1,
                    decoding_time,
                    sample_duration: 10,
                    sample_composition_time_offset: 0,
                    sample_flags: SampleFlags::default(),
                    sample_description_index: 1,
                },
                data.to_vec(),
            )
        };
        [sample(0, b"AB"), sample(10, b"CDE")]
    }

    /// Returns the boxes `muxer` has written, each whole
    fn output(muxer: &mut Muxer) -> Vec<Vec<u8>> {
        iter::from_fn(|| muxer.poll_output())
            .map(Output::into_bytes)
            .collect()
    }

    /// Returns the `moof` numbered `sequence_number` of [`two_samples`], whose `mdat` follows it
    fn two_samples_moof(sequence_number: u32) -> Vec<u8> {
        // moof 108 bytes, so the media data starts at 108 + 8
        written(
            b"moof",
            &[
                &written(b"mfhd", &[&[0; 4], &sequence_number.to_be_bytes()]),
                &written(
                    b"traf",
                    &[
                        &written(b"tfhd", &[&[0, 0x02, 0, 0], &1_u32.to_be_bytes()]),
                        &written(b"tfdt", &[&[0; 4], &0_u32.to_be_bytes()]),
                        &written(
                            b"trun",
                            &[
                                &[0, 0, 0x07, 0x01],
                                &2_u32.to_be_bytes(),
                                &116_i32.to_be_bytes(),
                                &10_u32.to_be_bytes(),
                                &2_u32.to_be_bytes(),
                                &[0; 4],
                                &10_u32.to_be_bytes(),
                                &3_u32.to_be_bytes(),
                                &[0; 4],
                            ],
                        ),
                    ],
                ),
            ],
        )
    }

    #[test]
    fn the_movie_of_a_fragmented_file_or_a_segment_declares_no_sample_and_has_a_trex() {
        let mut declaring = one_track();
        if let Some(trak) = declaring.trak.first_mut() {
            trak.mdia.minf.stbl = track(1, 10, &[2], &[(1, 1)], &[0]).mdia.minf.stbl;
        }

        for mut muxer in [
            Muxer::fragmented(FragmentedOptions::default()),
            Muxer::segment(SegmentOptions::default()),
        ] {
            assert_eq!(muxer.movie(), None);
            muxer.handle_movie(declaring.clone()).unwrap();
            assert_eq!(
                muxer.movie(),
                Some(&one_fragmented_track()),
                "{:?}",
                muxer.file_structure()
            );
        }
    }

    #[test]
    fn the_movie_of_a_finished_file_that_is_not_fragmented_is_the_moov_it_wrote_without_an_mvex() {
        let mut muxer = Muxer::non_fragmented(NonFragmentedOptions::default());
        muxer.handle_movie(one_fragmented_track()).unwrap();
        muxer.handle_samples(two_samples()).unwrap();
        muxer.finish().unwrap();

        let output = output(&mut muxer);
        let moov = MovieBox::decode(output.last().unwrap()).unwrap().0;
        assert_eq!((muxer.movie(), moov.mvex.as_ref()), (Some(&moov), None));
    }

    #[test]
    fn a_fragmented_file_is_its_brands_its_movie_then_a_movie_fragment_and_its_media_data() {
        let mut muxer = Muxer::fragmented(FragmentedOptions::default());

        muxer
            .handle_file_type(file_type(b"iso6", &[b"iso6", b"mp41"]))
            .unwrap();
        muxer.handle_movie(one_fragmented_track()).unwrap();
        muxer.handle_samples(two_samples()).unwrap();
        muxer.finish().unwrap();

        assert_eq!(
            output(&mut muxer),
            [
                written(b"ftyp", &[b"iso6", &[0; 4], b"iso6", b"mp41"]),
                encoded(&one_fragmented_track()),
                two_samples_moof(1),
                written(b"mdat", &[b"ABCDE"]),
            ]
        );
    }

    #[test]
    fn the_mfra_that_ends_a_fragmented_file_lists_where_each_fragment_starts_and_its_mfro_finds_it()
    {
        let mut muxer = Muxer::fragmented({
            let mut options = FragmentedOptions::default();
            options.movie_fragment_random_access = true;
            options
        });
        muxer
            .handle_file_type(file_type(b"iso6", &[b"iso6"]))
            .unwrap();
        muxer.handle_movie(one_fragmented_track()).unwrap();
        let [first, second] = two_samples();
        muxer.handle_samples([first]).unwrap();
        muxer.handle_samples([second]).unwrap();
        muxer.finish().unwrap();
        let boxes = output(&mut muxer);
        let file = boxes.concat();
        let offset_of = |index: usize| to_u64(boxes.iter().take(index).map(Vec::len).sum());

        let (mut demuxer, _) = demux(&file);
        let mfra = demuxer.movie_fragment_random_access().unwrap().clone();
        let resumed: Vec<(u64, Option<u64>)> = mfra
            .tfra
            .iter()
            .flat_map(|tfra| &tfra.entries)
            .map(|entry| {
                demuxer.resume_at(entry.moof_offset).unwrap();
                let first = read(&mut demuxer, &file).into_iter().next();
                (
                    entry.moof_offset,
                    first.map(|sample| sample.properties().decoding_time),
                )
            })
            .collect();

        assert_eq!(resumed, [(offset_of(2), Some(0)), (offset_of(4), Some(10))]);
        assert_eq!(
            movie_fragment_random_access_start(to_u64(file.len()), file.last_chunk().unwrap()),
            Some(offset_of(6))
        );
    }

    /// Reads `file` with a [`Demuxer`] from wherever it wants, and returns the demuxer and the samples it returned
    fn demux(file: &[u8]) -> (Demuxer, Vec<Sample>) {
        let mut demuxer = Demuxer::new();
        let samples = read(&mut demuxer, file);
        (demuxer, samples)
    }

    /// Reads `file` with `demuxer` from wherever it wants to the end, and returns the samples it returned
    fn read(demuxer: &mut Demuxer, file: &[u8]) -> Vec<Sample> {
        let mut samples = Vec::new();
        while let Some(wanted) = demuxer.wanted_input() {
            match file.get(usize::try_from(wanted.offset()).unwrap()..) {
                Some(input) if !input.is_empty() => {
                    demuxer.handle_input(wanted.offset(), input).unwrap()
                }
                Some(_) | None => demuxer.finish().unwrap(),
            };
            samples.extend(iter::from_fn(|| demuxer.poll_sample()));
        }
        samples
    }

    /// Returns the boxes `muxer` writes of `ftyp`, `movie` and `samples`, each whole
    fn mux(
        mut muxer: Muxer,
        ftyp: FileTypeBox,
        movie: MovieBox,
        samples: Vec<Sample>,
    ) -> Vec<Vec<u8>> {
        muxer.handle_file_type(ftyp).unwrap();
        muxer.handle_movie(movie).unwrap();
        muxer.handle_samples(samples).unwrap();
        muxer.finish().unwrap();
        output(&mut muxer)
    }

    #[test]
    fn samples_read_past_a_short_stts_write_back_as_a_file_read_without_a_leniency() {
        let brands = file_type(b"isom", &[b"isom", b"mp41"]);
        let written = mux(
            Muxer::non_fragmented(NonFragmentedOptions::default()),
            brands.clone(),
            one_track(),
            two_samples().to_vec(),
        );
        let (moov, head) = written.split_last().unwrap();
        let mut movie = MovieBox::decode(moov).unwrap().0;
        if let Some(stts) = movie
            .trak
            .first_mut()
            .and_then(|trak| trak.mdia.minf.stbl.stts.as_mut())
        {
            stts.entries
                .iter_mut()
                .for_each(|entry| entry.sample_count = 1);
        }
        let (lenient, read) = demux(&[head.concat(), encoded(&movie)].concat());
        assert_eq!(
            lenient.leniencies().collect::<Vec<_>>(),
            [&Leniency::SampleTable {
                track_id: 1,
                box_type: TimeToSampleBox::BOX_TYPE,
                kind: SampleTableDisagreement::Short { missing_samples: 1 },
            }]
        );

        let (strict, read_again) = demux(
            &mux(
                Muxer::non_fragmented(NonFragmentedOptions::default()),
                brands,
                one_track(),
                read.clone(),
            )
            .concat(),
        );

        assert_eq!(
            (strict.leniencies().count(), read_again, read),
            (0, two_samples().to_vec(), two_samples().to_vec())
        );
    }

    #[test]
    fn the_padding_and_the_degradation_priority_of_samples_carry_from_movie_fragments_into_sample_tables()
     {
        let samples = two_samples().map(|sample| {
            let mut properties = *sample.properties();
            properties.sample_flags.sample_padding_value = 5;
            properties.sample_flags.sample_degradation_priority =
                u16::try_from(properties.decoding_time).unwrap();
            Sample::new(properties, sample.into_data())
        });

        let fragmented = mux(
            Muxer::fragmented(FragmentedOptions::default()),
            file_type(b"iso6", &[b"iso6", b"mp41"]),
            one_fragmented_track(),
            samples.to_vec(),
        );
        let not_fragmented = mux(
            Muxer::non_fragmented(NonFragmentedOptions::default()),
            file_type(b"isom", &[b"isom", b"mp41"]),
            one_track(),
            demux(&fragmented.concat()).1,
        );

        assert_eq!(demux(&not_fragmented.concat()).1, samples);
    }

    #[test]
    fn a_segment_is_its_brands_then_a_movie_fragment_and_its_media_data_of_the_movie_of_its_initialization_segment()
     {
        let mut muxer = Muxer::segment({
            let mut options = SegmentOptions::default();
            options.first_sequence_number = 5;
            options
        });

        muxer
            .handle_file_type(file_type(b"msdh", &[b"msdh", b"iso6"]))
            .unwrap();
        muxer.handle_movie(one_fragmented_track()).unwrap();
        muxer.handle_samples(two_samples()).unwrap();
        muxer.finish().unwrap();

        assert_eq!(
            output(&mut muxer),
            [
                written(b"styp", &[b"msdh", &[0; 4], b"msdh", b"iso6"]),
                two_samples_moof(5),
                written(b"mdat", &[b"ABCDE"]),
            ]
        );
    }

    #[test]
    fn an_indexed_segment_reserves_its_sidx_after_its_brands_and_writes_it_over_the_reservation_as_it_finishes()
     {
        let mut muxer = Muxer::segment({
            let mut options = SegmentOptions::default();
            options.first_sequence_number = 5;
            options.segment_index = Some(2);
            options
        });
        muxer
            .handle_file_type(file_type(b"msdh", &[b"msdh", b"iso6"]))
            .unwrap();
        muxer.handle_movie(one_fragmented_track()).unwrap();
        let [first, second] = two_samples();
        muxer.handle_samples([first]).unwrap();
        muxer.handle_samples([second]).unwrap();
        let before_finish: Vec<Output> = iter::from_fn(|| muxer.poll_output()).collect();
        muxer.finish().unwrap();
        let finishing: Vec<Output> = iter::from_fn(|| muxer.poll_output()).collect();
        let segment = written_at([before_finish.clone(), finishing.clone()].concat());
        let initialization = mux(
            Muxer::fragmented(FragmentedOptions::default()),
            file_type(b"iso6", &[b"iso6"]),
            one_fragmented_track(),
            Vec::new(),
        )
        .concat();
        let in_file = |output: &Output| to_u64(initialization.len()) + output.offset();
        let box_types = |names: [&[u8; 4]; 6]| names.map(|name| BoxType::compact(*name)).to_vec();

        let (demuxer, samples) = demux(&[initialization.as_slice(), &segment].concat());

        assert_eq!(
            (
                top_level(&written_at(before_finish.clone())),
                top_level(&segment)
            ),
            (
                box_types([b"styp", b"free", b"moof", b"mdat", b"moof", b"mdat"]),
                box_types([b"styp", b"sidx", b"moof", b"mdat", b"moof", b"mdat"])
            )
        );
        assert_eq!(
            finishing
                .iter()
                .map(|output| (output.offset(), output.bytes().len()))
                .collect::<Vec<_>>(),
            before_finish
                .get(1)
                .map(|free| (free.offset(), free.bytes().len()))
                .into_iter()
                .collect::<Vec<_>>()
        );
        let subsegments: Vec<_> = demuxer
            .segment_indexes()
            .iter()
            .flat_map(|index| index.subsegments())
            .map(|subsegment| (subsegment.byte_range(), subsegment.presentation_time()))
            .collect();
        let [first_moof, second_moof] =
            [2, 4].map(|index| in_file(before_finish.get(index).unwrap()));
        let end = to_u64(initialization.len() + segment.len());
        assert_eq!(
            subsegments,
            [(first_moof..second_moof, 0..10), (second_moof..end, 10..20)]
        );
        assert_eq!(samples, two_samples());
    }

    #[test]
    fn calls_past_the_subsegments_of_an_indexed_segment_are_refused_and_fewer_leave_its_free_box() {
        let indexed = |subsegments| {
            let mut muxer = Muxer::segment({
                let mut options = SegmentOptions::default();
                options.segment_index = Some(subsegments);
                options
            });
            muxer.handle_file_type(file_type(b"iso6", &[])).unwrap();
            muxer.handle_movie(one_fragmented_track()).unwrap();
            muxer
        };
        let [first, second] = two_samples();
        let mut past = indexed(1);
        past.handle_samples([first.clone()]).unwrap();
        let written: Vec<Output> = iter::from_fn(|| past.poll_output()).collect();
        let mut fewer = indexed(2);
        fewer.handle_samples([first]).unwrap();

        let refused = past.handle_samples([second]);
        let finished = fewer.finish();
        let segment = written_at(iter::from_fn(|| fewer.poll_output()));

        assert_eq!(
            (refused, past.poll_output(), finished),
            (
                Err(MuxError::from(WriteError::SegmentIndexCountMismatch {
                    expected_subsegments: 1,
                    written_subsegments: 2
                })),
                None,
                Err(MuxError::from(WriteError::SegmentIndexCountMismatch {
                    expected_subsegments: 2,
                    written_subsegments: 1
                })),
            )
        );
        assert_eq!(written.len(), 4);
        assert_eq!(
            top_level(&segment),
            [b"styp", b"free", b"moof", b"mdat"].map(|name| BoxType::compact(*name))
        );
    }

    #[test]
    fn a_file_that_is_not_fragmented_is_its_brands_its_media_data_then_the_movie_that_declares_it()
    {
        let mut muxer = Muxer::non_fragmented(NonFragmentedOptions::default());

        muxer
            .handle_file_type(file_type(b"isom", &[b"isom", b"mp41"]))
            .unwrap();
        muxer.handle_movie(one_track()).unwrap();
        muxer.handle_samples(two_samples()).unwrap();
        muxer.finish().unwrap();

        let ftyp = written(b"ftyp", &[b"isom", &[0; 4], b"isom", b"mp41"]);
        let mut expected = one_track();
        expected.mvhd.duration = 200;
        if let Some(trak) = expected.trak.first_mut() {
            trak.tkhd.duration = 200;
            trak.mdia.mdhd.duration = 20;
            let stbl = &mut trak.mdia.minf.stbl;
            let mut stts = TimeToSampleBox::default();
            let mut run = TimeToSampleEntry::default();
            run.sample_count = 2;
            run.sample_delta = 10;
            stts.entries = Vec::from([run]);
            stbl.stts = Some(stts);
            let mut stsc = SampleToChunkBox::default();
            let mut chunk = SampleToChunkEntry::default();
            chunk.first_chunk = 1;
            chunk.samples_per_chunk = 2;
            chunk.sample_description_index = 1;
            stsc.entries = Vec::from([chunk]);
            stbl.stsc = Some(stsc);
            let mut stsz = SampleSizeBox::default();
            stsz.sample_sizes = SampleSizes::Table {
                entry_sizes: Vec::from([2, 3]),
            };
            stbl.sample_size = Some(SampleSizeVariant::SampleSize(stsz));
            let mut stco = ChunkOffsetBox::default();
            stco.chunk_offsets = Vec::from([u32::try_from(ftyp.len()).unwrap() + 8]);
            stbl.chunk_offset = Some(ChunkOffsetVariant::ChunkOffset(stco));
        }
        assert_eq!(
            output(&mut muxer),
            [ftyp, written(b"mdat", &[b"ABCDE"]), encoded(&expected)]
        );
    }

    #[test]
    fn each_output_goes_to_the_end_of_the_outputs_before_it_in_every_file_structure() {
        let structures = [
            (
                Muxer::non_fragmented(NonFragmentedOptions::default()),
                one_track(),
            ),
            (
                Muxer::fragmented({
                    let mut options = FragmentedOptions::default();
                    options.movie_fragment_random_access = true;
                    options
                }),
                one_fragmented_track(),
            ),
            (
                Muxer::segment(SegmentOptions::default()),
                one_fragmented_track(),
            ),
        ];

        for (mut muxer, movie) in structures {
            muxer.handle_file_type(file_type(b"iso6", &[])).unwrap();
            muxer.handle_movie(movie).unwrap();
            let [first, second] = two_samples();
            muxer.handle_samples([first]).unwrap();
            muxer.handle_samples([second]).unwrap();
            muxer.finish().unwrap();
            let outputs: Vec<Output> = iter::from_fn(|| muxer.poll_output()).collect();

            let starts: Vec<u64> = outputs
                .iter()
                .scan(0, |end, output| {
                    let offset = *end;
                    *end += to_u64(output.bytes().len());
                    Some(offset)
                })
                .collect();
            let offsets: Vec<u64> = outputs.iter().map(Output::offset).collect();
            assert_eq!(offsets, starts);
        }
    }

    /// Returns the muxer of a recoverable file
    fn recoverable() -> Muxer {
        Muxer::non_fragmented(NonFragmentedOptions { recoverable: true })
    }

    /// Writes `outputs` where each says, and returns the file
    fn written_at(outputs: impl IntoIterator<Item = Output>) -> Vec<u8> {
        let mut file = Vec::new();
        for output in outputs {
            let start = usize::try_from(output.offset()).unwrap();
            let end = start.checked_add(output.bytes().len()).unwrap();
            if file.len() < end {
                file.resize(end, 0);
            }
            file.get_mut(start..end)
                .unwrap()
                .copy_from_slice(output.bytes());
        }
        file
    }

    /// Returns the types of the boxes at file level of `file`
    fn top_level(file: &[u8]) -> Vec<BoxType> {
        Boxes::new(file)
            .map(|framed| framed.unwrap().header().box_type())
            .collect()
    }

    #[test]
    fn a_recoverable_file_reads_as_fragmented_until_it_finishes_and_as_not_fragmented_after() {
        let mut muxer = recoverable();
        muxer.handle_file_type(file_type(b"iso6", &[])).unwrap();
        muxer.handle_movie(one_track()).unwrap();
        let [first, second] = two_samples();
        muxer.handle_samples([first.clone()]).unwrap();
        muxer.handle_samples([second.clone()]).unwrap();
        let before_finish: Vec<Output> = iter::from_fn(|| muxer.poll_output()).collect();
        muxer.finish().unwrap();
        let after: Vec<Output> = iter::from_fn(|| muxer.poll_output()).collect();

        let fragmented = written_at(before_finish.clone());
        let laid_out = written_at([before_finish, after].concat());
        let types = |names: &[&[u8; 4]]| {
            names
                .iter()
                .map(|&&name| BoxType::compact(name))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            (top_level(&fragmented), demux(&fragmented).1),
            (
                types(&[
                    b"ftyp", b"free", b"moov", b"moof", b"mdat", b"moof", b"mdat"
                ]),
                Vec::from([first.clone(), second.clone()])
            )
        );
        let (demuxer, samples) = demux(&laid_out);
        assert_eq!(
            (
                top_level(&laid_out),
                samples,
                demuxer.movie().and_then(|movie| movie.mvex.clone())
            ),
            (
                types(&[b"ftyp", b"free", b"mdat", b"moov"]),
                Vec::from([first, second]),
                None
            )
        );
        assert_eq!(
            (
                muxer.file_structure(),
                muxer.movie().is_some_and(|movie| movie.mvex.is_none())
            ),
            (FileStructure::NonFragmented, true)
        );
    }

    #[test]
    fn a_recoverable_file_past_the_held_locations_stays_fragmented_and_whole() {
        let mut muxer = recoverable().with_limits(MuxLimits::new().with_held_location_count(1));
        muxer.handle_file_type(file_type(b"iso6", &[])).unwrap();
        muxer.handle_movie(one_track()).unwrap();
        let samples = two_samples();
        for sample in samples.clone() {
            muxer.handle_samples([sample]).unwrap();
        }

        let finished = [muxer.finish(), muxer.finish()];
        let file = written_at(iter::from_fn(|| muxer.poll_output()));

        assert_eq!(
            (finished, muxer.file_structure(), demux(&file).1),
            (
                [
                    Err(MuxError::NotFinalised {
                        held_locations: 2,
                        limit_locations: 1
                    }),
                    Err(MuxError::from(WriteError::AlreadyFinished))
                ],
                FileStructure::Fragmented,
                Vec::from(samples)
            )
        );
    }

    #[test]
    fn a_recoverable_file_whose_samples_the_sample_tables_refuse_stays_fragmented() {
        let mut muxer = recoverable();
        muxer.handle_file_type(file_type(b"iso6", &[])).unwrap();
        muxer.handle_movie(one_track()).unwrap();
        let [first, mut second] = two_samples();
        let mut properties = *second.properties();
        properties.decoding_time = 50;
        second = Sample::new(properties, second.into_data());
        muxer.handle_samples([first.clone()]).unwrap();
        muxer.handle_samples([second.clone()]).unwrap();

        let finished = muxer.finish();
        let file = written_at(iter::from_fn(|| muxer.poll_output()));

        assert_eq!(
            (finished, muxer.file_structure(), demux(&file).1),
            (
                Err(MuxError::from(WriteError::DecodingTimeMismatch {
                    track_id: 1,
                    expected_decoding_time: 10,
                    decoding_time: 50
                })),
                FileStructure::Fragmented,
                Vec::from([first, second])
            )
        );
    }

    #[test]
    fn the_brands_of_a_recoverable_file_name_one_from_iso5_on() {
        assert_eq!(
            recoverable().handle_file_type(file_type(b"isom", &[b"mp41"])),
            Err(MuxError::UnsupportedBrand {
                brand: FourCc::new(*b"isom")
            })
        );
    }

    #[test]
    fn samples_refused_leave_a_file_that_is_not_fragmented_as_it_was() {
        let written = |refused: &[Sample]| {
            let mut muxer = Muxer::non_fragmented(NonFragmentedOptions::default());
            muxer.handle_movie(one_track()).unwrap();
            let refusal = muxer.handle_samples(refused.iter().cloned());
            muxer.handle_samples(two_samples()).unwrap();
            muxer.finish().unwrap();
            (refusal, output(&mut muxer))
        };
        let [first, second] = two_samples();
        let mut properties = *second.properties();
        properties.track_id = 2;
        let of_no_track = Sample::new(properties, second.into_data());

        assert_eq!(
            written(&[first, of_no_track]),
            (
                Err(MuxError::from(WriteError::UnknownTrackId { track_id: 2 })),
                written(&[]).1
            )
        );
    }

    #[test]
    fn calls_out_of_the_order_of_the_file_are_refused() {
        let fragmented = || Muxer::fragmented(FragmentedOptions::default());
        let mut no_movie = fragmented();
        let mut twice = fragmented();
        twice.handle_movie(one_fragmented_track()).unwrap();
        let mut finished = fragmented();
        finished.handle_movie(one_fragmented_track()).unwrap();
        finished.finish().unwrap();
        let mut brands_twice = fragmented();
        brands_twice
            .handle_file_type(file_type(b"iso6", &[]))
            .unwrap();

        assert_eq!(
            [
                no_movie.handle_samples(two_samples()),
                no_movie.finish(),
                twice.handle_movie(one_fragmented_track()),
                twice.handle_file_type(file_type(b"iso6", &[])),
                brands_twice.handle_file_type(file_type(b"iso6", &[])),
                finished.handle_samples(two_samples()),
                finished.finish(),
            ],
            [
                Err(MuxError::MissingMovie),
                Err(MuxError::MissingMovie),
                Err(MuxError::DuplicateBox {
                    box_type: MovieBox::BOX_TYPE
                }),
                Err(MuxError::BoxOutOfOrder {
                    box_type: FileTypeBox::BOX_TYPE
                }),
                Err(MuxError::DuplicateBox {
                    box_type: FileTypeBox::BOX_TYPE
                }),
                Err(MuxError::from(WriteError::AlreadyFinished)),
                Err(MuxError::from(WriteError::AlreadyFinished)),
            ]
        );
    }

    #[test]
    fn brands_that_name_none_from_iso5_on_are_refused_for_a_fragmented_file() {
        let refused = |ftyp| Muxer::fragmented(FragmentedOptions::default()).handle_file_type(ftyp);

        assert_eq!(
            [
                refused(file_type(
                    b"iso5",
                    &[b"avc1", b"iso6", b"iso4", b"iso5", b"dash"]
                )),
                refused(file_type(b"mp42", &[b"mp42", b"isom", b"iso2"])),
                Muxer::non_fragmented(NonFragmentedOptions::default())
                    .handle_file_type(file_type(b"isom", &[b"isom"])),
            ],
            [
                Ok(()),
                Err(MuxError::UnsupportedBrand {
                    brand: FourCc::new(*b"mp42")
                }),
                Ok(()),
            ]
        );
    }

    /// Files of a movie of one track, the media data of its chunks in decoding order and out of it
    const FILES: [&[u8]; 2] = [
        include_bytes!("../../isobmff/tests/data/movie.mp4"),
        include_bytes!("../../isobmff/tests/data/out_of_order.mp4"),
    ];

    /// Returns the `moov` of `file`, which comes right after its `ftyp`
    fn movie_of(file: &[u8]) -> MovieBox {
        let after_ftyp = FileTypeBox::decode(file).unwrap().1;
        MovieBox::decode(after_ftyp).unwrap().0
    }

    /// Returns the locations of the samples of `movie` in decoding order
    fn locations_of(movie: &MovieBox) -> Vec<SampleLocation> {
        StructureDataReader::new(StructureDataLimits {
            sample_count: u64::MAX,
            segment_index_count: u64::MAX,
            subsegment_count: u64::MAX,
            leniency_count: u64::MAX,
            identified_media_data_count: u64::MAX,
            unresolved_sample_count: u64::MAX,
            compressed_box_count: u64::MAX,
        })
        .handle_movie(movie.clone())
        .unwrap()
        .into_decoding_order()
    }

    #[test]
    fn the_moov_written_from_the_locations_of_samples_written_is_the_moov_written_from_the_samples()
    {
        for file in FILES {
            let movie = movie_of(file);
            let samples = locations_of(&movie).into_iter().map(|location| {
                let range = location.byte_range();
                let data = file.get(
                    usize::try_from(range.start).unwrap()..usize::try_from(range.end).unwrap(),
                );
                Sample::new(*location.properties(), data.unwrap().to_vec())
            });
            let mut muxer = Muxer::non_fragmented(NonFragmentedOptions::default());
            muxer.handle_movie(movie.clone()).unwrap();
            muxer.handle_samples(samples).unwrap();
            muxer.finish().unwrap();
            let written: Vec<Vec<u8>> = iter::from_fn(|| muxer.poll_output())
                .map(Output::into_bytes)
                .collect();
            let written_moov = written.last().unwrap();

            let mut writer = SampleTableWriter::new(movie).unwrap();
            writer
                .handle_locations(&locations_of(&MovieBox::decode(written_moov).unwrap().0))
                .unwrap();

            assert_eq!(&encoded(writer.finish()), written_moov);
        }
    }
}
