//! [`Muxer`], which writes samples into a movie file or a segment

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use isobmff_boxes::{FileTypeBox, MovieBox, SegmentTypeBox};
use isobmff_core::{BoxDefinition, BoxEncode, BoxHeader, Error, FourCc};

use crate::error::MuxError;
use crate::file_structure::FileStructure;
use crate::framing::{BoxEvent, BoxWriter, to_u64};
use crate::media_data::MediaDataWriter;
use crate::sample::Sample;
use crate::structure_data::{MovieFragmentWriter, SampleTableWriter, StructureDataWriter};

/// Brands under which a track fragment may take the first byte of its `moof` as its base data offset, ISO/IEC 14496-12 §8.8.7.1 and Annex E
const DEFAULT_BASE_IS_MOOF_BRANDS: [[u8; 4]; 9] = [
    *b"iso5", *b"iso6", *b"iso7", *b"iso8", *b"iso9", *b"isoa", *b"isob", *b"isoc", *b"isod",
];

/// Brands earlier than `iso5`, under which a track fragment shall not take its `moof` as its base data offset, §8.8.7.1
const EARLIER_BRANDS: [[u8; 4]; 6] = [*b"isom", *b"avc1", *b"iso2", *b"mp71", *b"iso3", *b"iso4"];

/// Writer of a movie file or a segment from the samples a caller hands it
///
/// The muxer does no I/O. It takes the brands, the movie and the samples
/// of the file, and returns the file a box at file level at a time from
/// [`poll_output`](Self::poll_output), in the order they lie in the file:
/// writing them one after another gives the file. How the samples are laid
/// out is the [`FileStructure`] the muxer is made with.
///
/// The movie is the caller's `moov`, its tracks and their sample entries
/// already in place, its sample tables empty. The muxer declares the
/// samples in it, or in movie fragments, and refuses samples a reader of
/// the file would not read: those of a track, a sample entry or a data
/// reference entry in the file itself the movie does not declare.
///
/// A refused call leaves the muxer as it was.
///
/// # Examples
///
/// ```
/// use isobmff::boxes::{DataEntry, DataEntryUrlBox, MovieBox, SampleDescription, SampleEntry, TrackBox};
/// use isobmff::{DemuxEvent, Demuxer, FileStructure, FourCc, Muxer, Sample, SampleProperties};
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
/// let mut muxer = Muxer::new(FileStructure::NonFragmented);
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
/// let file: Vec<u8> = core::iter::from_fn(|| muxer.poll_output()).flatten().collect();
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
    file_structure: FileStructure,
    boxes: BoxWriter,
    structure_data: Option<StructureDataWriter>,
    media_data: MediaDataWriter,
    /// Boxes written and not yet polled, in the order they lie in the file
    output: VecDeque<Vec<u8>>,
    /// Whether the `ftyp`, or the `styp` of a segment, is written
    file_type: bool,
    finished: bool,
}

impl Muxer {
    /// Creates the muxer of a file laid out as `file_structure`
    #[must_use]
    pub fn new(file_structure: FileStructure) -> Self {
        Self {
            file_structure,
            boxes: BoxWriter::new(),
            structure_data: None,
            media_data: MediaDataWriter,
            output: VecDeque::new(),
            file_type: false,
            finished: false,
        }
    }

    /// Writes the brands of the file: its `ftyp`, or the `styp` of a segment, of the same brands (§4.3, §8.14.2)
    ///
    /// A file of movie fragments, or a segment, takes the first byte of each
    /// `moof` as the base data offset of its track fragments, which brands
    /// earlier than `iso5` do not allow (§8.8.7.1): its brands name one of
    /// `iso5` and later, and none earlier.
    ///
    /// # Errors
    ///
    /// * [`DuplicateBox`](MuxError::DuplicateBox): the brands are written
    ///   already.
    /// * [`BoxOutOfOrder`](MuxError::BoxOutOfOrder): a box is written
    ///   already, which the brands go before.
    /// * [`UnsupportedBrand`](MuxError::UnsupportedBrand): the file is
    ///   fragmented or a segment, and the brands name a brand earlier than
    ///   `iso5`, or none of `iso5` and later.
    /// * [`AlreadyFinished`](MuxError::AlreadyFinished): the file is finished.
    pub fn handle_file_type(&mut self, file_type: FileTypeBox) -> Result<(), MuxError> {
        let box_type = match self.file_structure {
            FileStructure::Segment { .. } => SegmentTypeBox::BOX_TYPE,
            FileStructure::Fragmented { .. } | FileStructure::NonFragmented => {
                FileTypeBox::BOX_TYPE
            }
        };
        self.refuse_if_finished()?;
        if self.file_type {
            return Err(MuxError::DuplicateBox { box_type });
        }
        if self.boxes.next_offset() > 0 {
            return Err(MuxError::BoxOutOfOrder { box_type });
        }
        let bytes = match self.file_structure {
            FileStructure::NonFragmented => write_box(&mut self.boxes, &file_type)?,
            FileStructure::Fragmented { .. } => {
                refuse_earlier_brands(&file_type)?;
                write_box(&mut self.boxes, &file_type)?
            }
            FileStructure::Segment { .. } => {
                refuse_earlier_brands(&file_type)?;
                let mut styp = SegmentTypeBox::default();
                styp.major_brand = file_type.major_brand;
                styp.minor_version = file_type.minor_version;
                styp.compatible_brands = file_type.compatible_brands;
                write_box(&mut self.boxes, &styp)?
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
    /// initialization segment, which the samples are checked against.
    ///
    /// # Errors
    ///
    /// * [`DuplicateBox`](MuxError::DuplicateBox): the muxer holds a movie
    ///   already.
    /// * [`MissingTrackExtends`](MuxError::MissingTrackExtends): the file is
    ///   fragmented or a segment, and a track of the movie has no `trex`.
    /// * [`UnexpectedBox`](MuxError::UnexpectedBox): the file is not
    ///   fragmented, and the movie has an `mvex`.
    /// * [`SampleTableNotEmpty`](MuxError::SampleTableNotEmpty): a table of
    ///   the sample table of a track declares samples.
    /// * [`Box`](MuxError::Box): the `moov` does not write.
    /// * [`AlreadyFinished`](MuxError::AlreadyFinished): the file is finished.
    pub fn handle_movie(&mut self, movie: MovieBox) -> Result<(), MuxError> {
        self.refuse_if_finished()?;
        if self.structure_data.is_some() {
            return Err(MuxError::DuplicateBox {
                box_type: MovieBox::BOX_TYPE,
            });
        }
        let structure_data = match self.file_structure {
            FileStructure::Fragmented {
                first_sequence_number,
            } => {
                let writer = MovieFragmentWriter::new(movie, first_sequence_number)?;
                let bytes = write_box(&mut self.boxes, writer.movie())?;
                self.output.push_back(bytes);
                StructureDataWriter::Fragmented(writer)
            }
            FileStructure::Segment {
                first_sequence_number,
            } => StructureDataWriter::Fragmented(MovieFragmentWriter::new(
                movie,
                first_sequence_number,
            )?),
            FileStructure::NonFragmented => {
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
    /// * [`UnknownTrackId`](MuxError::UnknownTrackId),
    ///   [`UnknownSampleDescriptionIndex`](MuxError::UnknownSampleDescriptionIndex),
    ///   [`UnknownDataReferenceIndex`](MuxError::UnknownDataReferenceIndex),
    ///   [`ExternalDataReference`](MuxError::ExternalDataReference),
    ///   [`UnsupportedDataReference`](MuxError::UnsupportedDataReference): a
    ///   sample is of a track, a sample entry or a data reference entry in the
    ///   file itself the movie has none of.
    /// * [`MixedSampleDescriptions`](MuxError::MixedSampleDescriptions),
    ///   [`DecodingTimeMismatch`](MuxError::DecodingTimeMismatch): the
    ///   samples of a track are described by more than one sample entry, or
    ///   their decoding times do not follow from their durations.
    /// * [`SampleSizeOutOfRange`](MuxError::SampleSizeOutOfRange),
    ///   [`DataOffsetOutOfRange`](MuxError::DataOffsetOutOfRange): the media
    ///   data of a sample is longer, or lies further, than the structure-data
    ///   reaches.
    /// * [`AlreadyFinished`](MuxError::AlreadyFinished): the file is finished.
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
        match structure_data {
            StructureDataWriter::NonFragmented(writer) => {
                let data_offset = self
                    .boxes
                    .next_offset()
                    .saturating_add(header.encoded_len());
                writer.handle_samples(&samples, data_offset)?;
            }
            StructureDataWriter::Fragmented(writer) => {
                let moof = writer.movie_fragment(&samples, header.encoded_len())?;
                let bytes = write_box(&mut self.boxes, &moof)?;
                self.output.push_back(bytes);
            }
        }
        let bytes = self.media_data.write(&mut self.boxes, header, &samples)?;
        self.output.push_back(bytes);

        Ok(())
    }

    /// Returns the next box at file level written, in the order the boxes lie in the file, or `None` when the muxer has written none it has not returned
    pub fn poll_output(&mut self) -> Option<Vec<u8>> {
        self.output.pop_front()
    }

    /// Ends the file: writes the `moov` of a file that is not fragmented, with the sample tables of its samples
    ///
    /// The muxer then takes no more input. The boxes written are still
    /// returned by [`poll_output`](Self::poll_output).
    ///
    /// # Errors
    ///
    /// * [`MissingMovie`](MuxError::MissingMovie): the muxer holds no movie.
    /// * [`Box`](MuxError::Box): the `moov` does not write.
    /// * [`AlreadyFinished`](MuxError::AlreadyFinished): the file is finished.
    pub fn finish(&mut self) -> Result<(), MuxError> {
        self.refuse_if_finished()?;
        let structure_data = self.structure_data.take().ok_or(MuxError::MissingMovie)?;
        if let StructureDataWriter::NonFragmented(writer) = structure_data {
            let bytes = write_box(&mut self.boxes, &writer.finish())?;
            self.output.push_back(bytes);
        }
        self.boxes.finish()?;
        self.finished = true;

        Ok(())
    }

    /// Refuses a call once the file is finished
    const fn refuse_if_finished(&self) -> Result<(), MuxError> {
        if self.finished {
            return Err(MuxError::AlreadyFinished);
        }
        Ok(())
    }
}

/// Refuses `file_type` where its brands do not allow the `moof` as the base data offset of a track fragment, §8.8.7.1
fn refuse_earlier_brands(file_type: &FileTypeBox) -> Result<(), MuxError> {
    let brands = || iter_brands(file_type);
    if let Some(brand) = brands().find(|brand| EARLIER_BRANDS.contains(brand.as_bytes())) {
        return Err(MuxError::UnsupportedBrand { brand });
    }
    if !brands().any(|brand| DEFAULT_BASE_IS_MOOF_BRANDS.contains(brand.as_bytes())) {
        return Err(MuxError::UnsupportedBrand {
            brand: file_type.major_brand,
        });
    }
    Ok(())
}

/// Returns the major brand and the compatible brands of `file_type`
fn iter_brands(file_type: &FileTypeBox) -> impl Iterator<Item = FourCc> + '_ {
    core::iter::once(file_type.major_brand).chain(file_type.compatible_brands.iter().copied())
}

/// Writes `value` as a box at file level through `boxes`, and returns its bytes
///
/// # Errors
///
/// * [`Box`](MuxError::Box): the box does not write.
/// * The failures of [`BoxWriter::handle_event`].
fn write_box<Value: BoxEncode + BoxDefinition>(
    boxes: &mut BoxWriter,
    value: &Value,
) -> Result<Vec<u8>, MuxError> {
    let payload_len = value.payload_len();
    let too_long = MuxError::Box {
        error: Error::truncated_buffer(payload_len, to_u64(usize::MAX)),
    };
    let mut payload = alloc::vec![0; usize::try_from(payload_len).map_err(|_| too_long)?];
    value
        .encode_payload(&mut payload)
        .map_err(|error| MuxError::Box { error })?;
    let header = BoxHeader::with_payload_len(Value::BOX_TYPE, payload_len).ok_or(too_long)?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(boxes.handle_event(BoxEvent::Header(header))?);
    bytes.extend_from_slice(boxes.handle_event(BoxEvent::Payload(&payload))?);
    bytes.extend_from_slice(boxes.handle_event(BoxEvent::End)?);

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use core::iter;

    use isobmff_boxes::{
        ChunkOffsetBox, ChunkOffsetVariant, FileTypeBox, MovieBox, MovieExtendsBox, SampleFlags,
        SampleSizeBox, SampleSizeVariant, SampleSizes, SampleToChunkBox, SampleToChunkEntry,
        TimeToSampleBox, TimeToSampleEntry, TrackExtendsBox,
    };
    use isobmff_core::{BoxDefinition, FourCc};

    use super::Muxer;
    use crate::error::MuxError;
    use crate::file_structure::FileStructure;
    use crate::sample::{Sample, SampleProperties};
    use crate::structure_data::fixtures::{encoded, movie, track, written};

    /// Returns a movie of track 1 of no sample, the movie in a timescale of 1000 and the track of 100
    fn one_track() -> MovieBox {
        let mut trak = track(1, 0, &[], &[], &[]);
        trak.mdia.mdhd.timescale = 100;
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
        iter::from_fn(|| muxer.poll_output()).collect()
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
    fn a_fragmented_file_is_its_brands_its_movie_then_a_movie_fragment_and_its_media_data() {
        let mut muxer = Muxer::new(FileStructure::Fragmented {
            first_sequence_number: 1,
        });

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
    fn a_segment_is_its_brands_then_a_movie_fragment_and_its_media_data_of_the_movie_of_its_initialization_segment()
     {
        let mut muxer = Muxer::new(FileStructure::Segment {
            first_sequence_number: 5,
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
    fn a_file_that_is_not_fragmented_is_its_brands_its_media_data_then_the_movie_that_declares_it()
    {
        let mut muxer = Muxer::new(FileStructure::NonFragmented);

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
    fn calls_out_of_the_order_of_the_file_are_refused() {
        let fragmented = || {
            Muxer::new(FileStructure::Fragmented {
                first_sequence_number: 1,
            })
        };
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
                Err(MuxError::AlreadyFinished),
                Err(MuxError::AlreadyFinished),
            ]
        );
    }

    #[test]
    fn brands_that_do_not_allow_a_track_fragment_based_on_its_moof_are_refused_for_a_fragmented_file()
     {
        let refused = |ftyp| {
            Muxer::new(FileStructure::Fragmented {
                first_sequence_number: 1,
            })
            .handle_file_type(ftyp)
        };

        assert_eq!(
            [
                refused(file_type(b"iso6", &[b"iso6", b"isom"])),
                refused(file_type(b"mp42", &[b"mp41"])),
                Muxer::new(FileStructure::NonFragmented)
                    .handle_file_type(file_type(b"isom", &[b"isom"])),
            ],
            [
                Err(MuxError::UnsupportedBrand {
                    brand: FourCc::new(*b"isom")
                }),
                Err(MuxError::UnsupportedBrand {
                    brand: FourCc::new(*b"mp42")
                }),
                Ok(()),
            ]
        );
    }
}
