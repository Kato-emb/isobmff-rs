//! [`FragmentedMovieFileMuxer`], which writes a movie whose samples lie in movie fragments, and [`SegmentMuxer`], which writes it in segments

use alloc::vec::Vec;

use isobmff_boxes::{FileTypeBox, MovieBox, MovieExtendsBox, SegmentTypeBox, TrackExtendsBox};
use isobmff_core::BoxEncode;

use super::movie::{append_encoded, check_track, data_reference, movie_box, track_box};
use super::{Error, MovieDescription, MuxerConfig, TrackDescription};
use crate::framing;
use crate::structure_data::{self, MovieFragmentWriter, SampleTableWriter, TrackSampleDescription};
use crate::{SampleLocation, SampleProperties};

/// Where a fragmented muxer stands
#[derive(Debug, Clone)]
enum State {
    /// Before the first sample, while tracks are added and the `moov` is yet
    /// to be written
    Tracks {
        movie: MovieDescription,
        tracks: Vec<TrackDescription>,
    },
    /// After the `moov`, the samples of the fragment being written and their
    /// bytes
    Fragments {
        writer: MovieFragmentWriter,
        media_data: Vec<u8>,
    },
    Finished,
}

/// Writer of a fragmented movie file: an `ftyp`, a `moov` that declares the tracks and no sample, and the movie fragments that hold the samples, each a `moof` and the `mdat` that follows it
///
/// The structure of a fragmented movie file, ISO/IEC 14496-12 §8.8, with a
/// [`MovieExtendsBox`] (`mvex`), §8.8.1, of a [`TrackExtendsBox`] (`trex`),
/// §8.8.3, for each track.
///
/// The muxer takes values and bytes, and returns bytes:
/// [`add_track`](Self::add_track) takes the description of each track, and
/// [`handle_sample`](Self::handle_sample) what the structure-data declares of
/// each sample with its bytes, in decoding order within its track. The first
/// sample writes the `moov`. [`finish_fragment`](Self::finish_fragment) ends
/// a fragment and [`finish`](Self::finish) the last.
/// [`poll_output`](Self::poll_output) appends what the muxer holds, as one run
/// of bytes: the `ftyp` first, the `moov` once the first sample is taken, and
/// each fragment once it ends. A call that fails changes nothing.
///
/// The bytes of a fragment's samples are held until the fragment ends,
/// since its `moof` declares them and comes before them; where a fragment
/// ends is the caller's to choose, as is the order of the samples across
/// tracks: the muxer neither ends one of its own accord nor reorders them.
/// Each `moof` is written as [`MovieFragmentWriter`] writes it, with
/// `default-base-is-moof`, so the caller names `iso5`, or a later brand that
/// allows it, in the `ftyp`; the muxer does not check the brands. The
/// decoding time of the first sample of a track is that of the first `tfdt`
/// of the track.
///
/// A fragmented movie file written again:
///
/// ```
/// use isobmff::demuxer::Event;
/// use isobmff::muxer::{MovieDescription, TrackDescription};
/// use isobmff::{Demuxer, DemuxerConfig, FragmentedMovieFileMuxer, MuxerConfig};
///
/// // Read the `ftyp`, the `moov` and every sample of a file, noting where each fragment starts
/// let file: &[u8] = include_bytes!("../../tests/fixtures/fragmented.mp4");
/// let mut demuxer = Demuxer::new(DemuxerConfig::default());
/// let (mut file_type, mut movie, mut samples, mut fragment_starts) = (None, None, Vec::new(), Vec::new());
/// loop {
///     while let Some(event) = demuxer.poll_event() {
///         match event {
///             Event::FileType(ftyp) => file_type = Some(ftyp),
///             Event::Movie(moov) => {
///                 for trak in &moov.trak {
///                     demuxer.subscribe_track(trak.tkhd.track_id);
///                 }
///                 movie = Some(moov);
///             }
///             Event::MovieFragment(_) => fragment_starts.push(samples.len()),
///             Event::Sample(sample) => samples.push((sample.properties, sample.bytes.to_vec())),
///             _ => {}
///         }
///     }
///     let Some(required) = demuxer.required_input() else { break };
///     match file.get(usize::try_from(required.position)?..) {
///         Some(bytes) if !bytes.is_empty() => demuxer.handle_input(required.position, bytes),
///         _ => demuxer.finish(),
///     }
/// }
/// let (file_type, movie) = (file_type.ok_or("no ftyp")?, movie.ok_or("no moov")?);
///
/// // Write them in fragments of their own, ending each where one of the file ended
/// let mut muxer = FragmentedMovieFileMuxer::new(MovieDescription::from(&*movie), MuxerConfig::default(), file_type)?;
/// for trak in &movie.trak {
///     muxer.add_track(TrackDescription::from(trak))?;
/// }
/// let mut written = Vec::new();
/// for (index, (properties, bytes)) in samples.iter().enumerate() {
///     if index > 0 && fragment_starts.contains(&index) {
///         muxer.finish_fragment()?;
///     }
///     muxer.handle_sample(*properties, bytes)?;
///     muxer.poll_output(&mut written);
/// }
/// muxer.finish()?;
/// muxer.poll_output(&mut written);
///
/// // The file written holds the same samples
/// let mut demuxer = Demuxer::new(DemuxerConfig::default());
/// let mut read_back = Vec::new();
/// loop {
///     while let Some(event) = demuxer.poll_event() {
///         match event {
///             Event::Movie(moov) => {
///                 for trak in &moov.trak {
///                     demuxer.subscribe_track(trak.tkhd.track_id);
///                 }
///             }
///             Event::Sample(sample) => read_back.push((sample.properties, sample.bytes.to_vec())),
///             _ => {}
///         }
///     }
///     let Some(required) = demuxer.required_input() else { break };
///     match written.get(usize::try_from(required.position)?..) {
///         Some(bytes) if !bytes.is_empty() => demuxer.handle_input(required.position, bytes),
///         _ => demuxer.finish(),
///     }
/// }
/// assert_eq!(read_back, samples);
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct FragmentedMovieFileMuxer {
    config: MuxerConfig,
    state: State,
    /// Bytes [`poll_output`](Self::poll_output) has yet to append
    output: Vec<u8>,
}

impl FragmentedMovieFileMuxer {
    /// Creates a muxer of the movie `movie` describes, in a file that opens with `file_type`, and holds the `ftyp`
    ///
    /// # Errors
    /// * [`Error::BufferFull`]: the `ftyp` is longer than
    ///   [`MuxerConfig::buffer_limit`]; nothing is allocated for it.
    /// * [`Error::Encode`]: the `ftyp` does not write.
    pub fn new(
        movie: MovieDescription,
        config: MuxerConfig,
        file_type: FileTypeBox,
    ) -> Result<Self, Error> {
        let length = usize::try_from(file_type.encoded_len()).unwrap_or(usize::MAX);
        if length > config.buffer_limit {
            return Err(Error::BufferFull { length });
        }
        let mut output = Vec::with_capacity(length);
        append_encoded(&file_type, &mut output)?;
        Ok(Self {
            config,
            state: State::Tracks {
                movie,
                tracks: Vec::new(),
            },
            output,
        })
    }

    /// Adds the track `track` describes, before the first sample
    ///
    /// # Errors
    /// * [`Error::Finished`]: the muxer has finished.
    /// * [`Error::TrackAfterMediaData`]: a sample was taken.
    /// * [`Error::TrackId`]: the `track_ID` is 0 or that of a track added.
    /// * [`Error::DataReferenceIndex`]: a sample entry names another data
    ///   reference than 1, the media data in the same file.
    pub fn add_track(&mut self, track: TrackDescription) -> Result<(), Error> {
        match &mut self.state {
            State::Finished => Err(Error::Finished),
            State::Fragments { .. } => Err(Error::TrackAfterMediaData {
                track_id: track.track_id,
            }),
            State::Tracks { tracks, .. } => {
                check_track(tracks.iter(), &track)?;
                tracks.push(track);
                Ok(())
            }
        }
    }

    /// Takes the next sample of the fragment, with its bytes, and holds them until the fragment ends; the first sample writes the `moov` first
    ///
    /// The sample lies after the samples taken before it in the fragment.
    ///
    /// # Errors
    /// * [`Error::Finished`]: the muxer has finished.
    /// * [`Error::BufferFull`]: the bytes, with the `moov` before the first
    ///   sample, would grow what the muxer holds past
    ///   [`MuxerConfig::buffer_limit`]; the caller polls, ends the fragment
    ///   where it holds samples, polls again, and hands the sample over
    ///   again.
    /// * [`Error::UnknownTrack`]: no track of the sample's `track_ID` was added.
    /// * [`Error::StructureData`]: the fragment refuses the sample: it names a
    ///   sample entry the track does not have, its decoding time is not where
    ///   the samples of its track before it end, or the fragment holds as
    ///   many samples as [`MuxerConfig::sample_count_limit`], and the caller
    ///   ends it and hands the sample over again.
    /// * [`Error::Overflow`]: the sample is longer than a track fragment run
    ///   counts.
    /// * [`Error::Encode`]: the `moov` does not write, as where a sample
    ///   entry or a box of a description does not.
    pub fn handle_sample(
        &mut self,
        properties: SampleProperties,
        bytes: &[u8],
    ) -> Result<(), Error> {
        if matches!(self.state, State::Finished) {
            return Err(Error::Finished);
        }
        let size = u32::try_from(bytes.len()).map_err(|_| Error::Overflow)?;
        let buffer_limit = self.config.buffer_limit;
        match &mut self.state {
            State::Finished => Err(Error::Finished),
            State::Tracks { movie, tracks } => {
                let moov = fragmented_movie_box(movie, tracks);
                let length = self
                    .output
                    .len()
                    .saturating_add(usize::try_from(moov.encoded_len()).unwrap_or(usize::MAX))
                    .saturating_add(bytes.len());
                if length > buffer_limit {
                    return Err(Error::BufferFull { length });
                }
                let dref = data_reference();
                let descriptions: Vec<TrackSampleDescription<'_>> = tracks
                    .iter()
                    .map(|track| TrackSampleDescription {
                        track_id: track.track_id,
                        stsd: &track.sample_entries,
                        dref: &dref,
                    })
                    .collect();
                let mut writer =
                    MovieFragmentWriter::new(&descriptions, self.config.sample_count_limit);
                writer
                    .handle_location(SampleLocation {
                        properties,
                        offset: 0,
                        size,
                    })
                    .map_err(from_fragment)?;
                append_encoded(&moov, &mut self.output)?;
                self.state = State::Fragments {
                    writer,
                    media_data: bytes.to_vec(),
                };
                Ok(())
            }
            State::Fragments { writer, media_data } => {
                let length = self
                    .output
                    .len()
                    .saturating_add(media_data.len())
                    .saturating_add(bytes.len());
                if length > buffer_limit {
                    return Err(Error::BufferFull { length });
                }
                writer
                    .handle_location(SampleLocation {
                        properties,
                        offset: u64::try_from(media_data.len()).map_err(|_| Error::Overflow)?,
                        size,
                    })
                    .map_err(from_fragment)?;
                media_data.extend_from_slice(bytes);
                Ok(())
            }
        }
    }

    /// Ends the fragment, and holds its `moof` and its `mdat` for [`poll_output`](Self::poll_output)
    ///
    /// The fragment is written whatever [`MuxerConfig::buffer_limit`] holds.
    /// Where it holds no sample, nothing is written.
    ///
    /// # Errors
    /// * [`Error::Finished`]: the muxer has finished.
    /// * [`Error::Encode`]: the `moof` does not write.
    pub fn finish_fragment(&mut self) -> Result<(), Error> {
        self.write_fragment(None)
    }

    /// Appends the bytes the muxer holds to `out`, and holds none
    pub fn poll_output(&mut self, out: &mut Vec<u8>) {
        out.append(&mut self.output);
    }

    /// Ends the last fragment, or writes the `moov` where no sample was taken, and holds it for [`poll_output`](Self::poll_output)
    ///
    /// The muxer then takes nothing more: it ends the file, unlike the
    /// `finish` of a writer of structure-data, which ends one unit and goes on.
    ///
    /// # Errors
    /// * [`Error::Finished`]: the muxer has finished.
    /// * [`Error::Encode`]: the `moof` or the `moov` does not write.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.finish_after(None)
    }

    /// Ends the fragment, after `segment_type` where there is one, as [`finish_fragment`](Self::finish_fragment) does
    fn write_fragment(&mut self, segment_type: Option<&SegmentTypeBox>) -> Result<(), Error> {
        match &mut self.state {
            State::Finished => Err(Error::Finished),
            State::Tracks { .. } => Ok(()),
            State::Fragments { writer, .. } if !writer.has_samples() => Ok(()),
            State::Fragments { writer, media_data } => {
                let start = self.output.len();
                // Why finish a clone: a `moof` that does not write leaves the
                // muxer as it was, the samples of the fragment and all.
                let mut finished = writer.clone();
                let moof = finished.finish();
                let header = u64::try_from(media_data.len())
                    .ok()
                    .and_then(framing::media_data_header)
                    .ok_or(Error::Overflow)?;
                let written = segment_type
                    .map_or(Ok(()), |styp| append_encoded(styp, &mut self.output))
                    .and_then(|()| append_encoded(&moof, &mut self.output));
                if let Err(error) = written {
                    self.output.truncate(start);
                    return Err(error);
                }
                self.output.extend_from_slice(&header);
                self.output.append(media_data);
                *writer = finished;
                Ok(())
            }
        }
    }

    /// Ends the muxer, as [`finish`](Self::finish) does, the last fragment after `segment_type` where there is one
    fn finish_after(&mut self, segment_type: Option<&SegmentTypeBox>) -> Result<(), Error> {
        match &self.state {
            State::Finished => return Err(Error::Finished),
            State::Tracks { movie, tracks } => {
                let moov = fragmented_movie_box(movie, tracks);
                append_encoded(&moov, &mut self.output)?;
            }
            State::Fragments { .. } => self.write_fragment(segment_type)?,
        }
        self.state = State::Finished;
        Ok(())
    }
}

/// Writer of a movie in segments: an initialization segment, the `ftyp` and a `moov` that declares the tracks and no sample, and media segments, each an `styp` and a movie fragment
///
/// The segments of ISO/IEC 14496-12 §8.16, each media segment opening with a
/// [`SegmentTypeBox`] (`styp`), §8.16.2, and holding one movie fragment as
/// [`FragmentedMovieFileMuxer`] writes it.
///
/// The muxer is driven as a [`FragmentedMovieFileMuxer`] is, with
/// [`finish_segment`](Self::finish_segment) in place of `finish_fragment`.
/// The segments follow one another in one run of bytes, which reads as one
/// fragmented movie file; a caller that keeps each segment apart polls after
/// the first sample, for the initialization segment, and after each
/// [`finish_segment`](Self::finish_segment) and [`finish`](Self::finish), for
/// a media segment. Where a segment ends is the caller's to choose; the muxer
/// writes no segment index.
///
/// ```
/// use isobmff::muxer::{MovieDescription, TrackDescription};
/// use isobmff::{MuxerConfig, SampleProperties, SegmentMuxer};
/// use isobmff_boxes::{FileTypeBox, SampleFlags, SegmentTypeBox};
/// use isobmff_core::{BoxDecode, FourCc};
///
/// // A movie of one track, whose sample entry is that of a file
/// let file: &[u8] = include_bytes!("../../tests/fixtures/segment.mp4");
/// let (file_type, rest) = FileTypeBox::decode(file)?;
/// let (moov, _) = isobmff_boxes::MovieBox::decode(rest)?;
/// let mut segment_type = SegmentTypeBox::default();
/// segment_type.major_brand = FourCc::new(*b"msdh");
/// let mut muxer = SegmentMuxer::new(MovieDescription::from(&moov), MuxerConfig::default(), file_type, segment_type)?;
/// muxer.add_track(TrackDescription::from(&moov.trak[0]))?;
///
/// // Write two segments of one sample each, keeping each segment apart
/// let sample = |decoding_time| SampleProperties {
///     track_id: moov.trak[0].tkhd.track_id,
///     decoding_time,
///     sample_duration: 1024,
///     sample_composition_time_offset: 0,
///     sample_flags: SampleFlags::default(),
///     sample_description_index: 1,
/// };
/// let (mut initialization, mut segments) = (Vec::new(), Vec::new());
/// muxer.handle_sample(sample(0), b"abc")?;
/// muxer.poll_output(&mut initialization);
/// muxer.finish_segment()?;
/// muxer.handle_sample(sample(1024), b"de")?;
/// muxer.finish()?;
/// for _ in 0..2 {
///     let mut segment = Vec::new();
///     muxer.poll_output(&mut segment);
///     segments.push(segment);
/// }
///
/// // Each media segment opens with the `styp`
/// assert_eq!(&segments[0][4..8], b"styp");
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct SegmentMuxer {
    muxer: FragmentedMovieFileMuxer,
    segment_type: SegmentTypeBox,
}

impl SegmentMuxer {
    /// Creates a muxer of the movie `movie` describes, whose initialization segment opens with `file_type` and each media segment with `segment_type`, and holds the `ftyp`
    ///
    /// # Errors
    /// As [`FragmentedMovieFileMuxer::new`].
    pub fn new(
        movie: MovieDescription,
        config: MuxerConfig,
        file_type: FileTypeBox,
        segment_type: SegmentTypeBox,
    ) -> Result<Self, Error> {
        Ok(Self {
            muxer: FragmentedMovieFileMuxer::new(movie, config, file_type)?,
            segment_type,
        })
    }

    /// Adds the track `track` describes, before the first sample
    ///
    /// # Errors
    /// As [`FragmentedMovieFileMuxer::add_track`].
    pub fn add_track(&mut self, track: TrackDescription) -> Result<(), Error> {
        self.muxer.add_track(track)
    }

    /// Takes the next sample of the segment, with its bytes, and holds them until the segment ends; the first sample writes the initialization segment first
    ///
    /// # Errors
    /// As [`FragmentedMovieFileMuxer::handle_sample`].
    pub fn handle_sample(
        &mut self,
        properties: SampleProperties,
        bytes: &[u8],
    ) -> Result<(), Error> {
        self.muxer.handle_sample(properties, bytes)
    }

    /// Ends the media segment, and holds its `styp`, its `moof` and its `mdat` for [`poll_output`](Self::poll_output)
    ///
    /// Where the segment holds no sample, nothing is written.
    ///
    /// # Errors
    /// * [`Error::Finished`]: the muxer has finished.
    /// * [`Error::Encode`]: the `styp` or the `moof` does not write.
    pub fn finish_segment(&mut self) -> Result<(), Error> {
        self.muxer.write_fragment(Some(&self.segment_type))
    }

    /// Appends the bytes the muxer holds to `out`, and holds none
    pub fn poll_output(&mut self, out: &mut Vec<u8>) {
        self.muxer.poll_output(out);
    }

    /// Ends the last media segment, or writes the initialization segment where no sample was taken, and holds it for [`poll_output`](Self::poll_output)
    ///
    /// The muxer then takes nothing more: it ends the file, unlike the
    /// `finish` of a writer of structure-data, which ends one unit and goes on.
    ///
    /// # Errors
    /// As [`finish_segment`](Self::finish_segment), and
    /// [`Error::Encode`] where the `moov` does not write.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.muxer.finish_after(Some(&self.segment_type))
    }
}

/// Returns the `moov` of `movie` whose tracks `tracks` describe, which declares no sample and extends each track into movie fragments
fn fragmented_movie_box(movie: &MovieDescription, tracks: &[TrackDescription]) -> MovieBox {
    let mut trak = Vec::with_capacity(tracks.len());
    let mut mvex = MovieExtendsBox::default();
    for track in tracks {
        let stbl = SampleTableWriter::new(
            &TrackSampleDescription {
                track_id: track.track_id,
                stsd: &track.sample_entries,
                dref: &data_reference(),
            },
            0,
        )
        .finish();
        trak.push(track_box(track, stbl, movie.timescale));
        let mut trex = TrackExtendsBox::default();
        trex.track_id = track.track_id;
        trex.default_sample_description_index = 1;
        mvex.trex.push(trex);
    }
    let mut moov = movie_box(movie, trak);
    moov.mvex = Some(mvex);
    moov
}

/// The error of a muxer for the error a fragment refuses a sample or a `moof` with
fn from_fragment(error: structure_data::Error) -> Error {
    if let structure_data::Error::UnknownTrack { track_id } = error {
        Error::UnknownTrack { track_id }
    } else {
        Error::StructureData(error)
    }
}
