//! [`MovieFileMuxer`], which writes a movie file of one `mdat` and the `moov` that declares it

use alloc::vec::Vec;

use isobmff_boxes::FileTypeBox;
use isobmff_core::BoxEncode;

use super::movie::{append_encoded, check_track, data_reference, media_data_header};
use super::{Error, MuxerConfig};
use super::{MovieParameters, TrackParameters};
use crate::structure_data::{self, SampleTableWriter, TrackSampleDescription};
use crate::{SampleLocation, SampleProperties};

/// Length of the header of the `mdat`, written with a `largesize` so that its length can be written over in place
const MEDIA_DATA_HEADER_LENGTH: u64 = 16;

/// The header of the `mdat` a caller writes over the one first written, once a [`MovieFileMuxer`] has finished
///
/// The header of the `media data` box, ISO/IEC 14496-12 §8.1.1, of 16 bytes
/// with a `largesize`, with the length of every sample in the box.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MediaDataHeader {
    /// File offset of the header
    pub offset: u64,
    /// The bytes of the header, of the length of the header first written
    pub bytes: [u8; 16],
}

/// A track added, and what the muxer has taken of its samples
#[derive(Debug, Clone)]
struct Track {
    parameters: TrackParameters,
    samples: SampleTableWriter,
    /// Decoding time of the first sample taken, from which the sample table
    /// counts
    first_decoding_time: Option<u64>,
}

/// Writer of a movie file: an `ftyp`, one `mdat` that holds every sample, and the `moov` that declares them
///
/// The structure of a movie file, ISO/IEC 14496-12 §4.3, §8.1.1, §8.2 and
/// §8.5.
///
/// The muxer takes values and bytes, and returns bytes:
/// [`add_track`](Self::add_track) takes the parameters of each track, and
/// [`handle_sample`](Self::handle_sample) what the structure-data declares of
/// each sample with its bytes, in decoding order within its track.
/// [`finish`](Self::finish) builds the `moov`. [`poll_output`](Self::poll_output)
/// appends what the muxer holds, as one run of bytes: the `ftyp` and the
/// header of the `mdat` first, the bytes of each sample as it is taken, and
/// the `moov` once finished. A call that fails changes nothing.
///
/// The order of the samples across tracks, and so the chunks they fall in,
/// is the order the caller hands them over in: the muxer does not reorder
/// them, and samples of one track handed over one after another fall in one
/// chunk, which keeps the tables short. The length of the `mdat` is known
/// only once every sample is in it, so the caller writes the header `finish`
/// returns over the one first written; a caller that cannot seek back writes
/// a fragmented file instead.
///
/// A movie file written again:
///
/// ```
/// use isobmff::demuxer::Event;
/// use isobmff::muxer::{MovieParameters, TrackParameters};
/// use isobmff::{Demuxer, DemuxerConfig, MovieFileMuxer, MuxerConfig};
///
/// // Read the `ftyp`, the `moov` and every sample of a file
/// let file: &[u8] = include_bytes!("../../tests/fixtures/movie_moov_first.mp4");
/// let mut demuxer = Demuxer::new(DemuxerConfig::default());
/// let (mut file_type, mut movie, mut samples) = (None, None, Vec::new());
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
/// // Write them into a file of their own
/// let parameters = MovieParameters::from(&*movie);
/// let mut muxer = MovieFileMuxer::new(parameters, MuxerConfig::default(), file_type)?;
/// for trak in &movie.trak {
///     muxer.add_track(TrackParameters::from(trak))?;
/// }
/// let mut written = Vec::new();
/// for (properties, bytes) in &samples {
///     muxer.handle_sample(*properties, bytes)?;
///     muxer.poll_output(&mut written);
/// }
/// let header = muxer.finish()?;
/// muxer.poll_output(&mut written);
///
/// // Write the header of the `mdat` over the first
/// let offset = usize::try_from(header.offset)?;
/// written[offset..offset + header.bytes.len()].copy_from_slice(&header.bytes);
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
pub struct MovieFileMuxer {
    movie: MovieParameters,
    tracks: Vec<Track>,
    config: MuxerConfig,
    /// File offset of the header of the `mdat`
    media_data_offset: u64,
    /// File offset of the next sample
    position: u64,
    /// Bytes [`poll_output`](Self::poll_output) has yet to append
    output: Vec<u8>,
    finished: bool,
}

impl MovieFileMuxer {
    /// Creates a muxer of the movie `movie` describes, in a file that opens with `file_type`, and holds the bytes that open the file
    ///
    /// # Errors
    /// * [`Error::BufferFull`]: the `ftyp` and the header of the `mdat` are
    ///   longer than [`MuxerConfig::buffer_limit`]; nothing is allocated for
    ///   them.
    /// * [`Error::Encode`]: the `ftyp` does not write.
    pub fn new(
        movie: MovieParameters,
        config: MuxerConfig,
        file_type: FileTypeBox,
    ) -> Result<Self, Error> {
        let opening = file_type
            .encoded_len()
            .saturating_add(MEDIA_DATA_HEADER_LENGTH);
        let length = usize::try_from(opening).unwrap_or(usize::MAX);
        if length > config.buffer_limit {
            return Err(Error::BufferFull { length });
        }
        let mut output = Vec::with_capacity(length);
        append_encoded(&file_type, &mut output)?;
        let media_data_offset = u64::try_from(output.len()).map_err(|_| Error::Overflow)?;
        let header = media_data_header(0).ok_or(Error::Overflow)?;
        output.extend_from_slice(&header);
        Ok(Self {
            movie,
            tracks: Vec::new(),
            config,
            media_data_offset,
            position: media_data_offset.saturating_add(MEDIA_DATA_HEADER_LENGTH),
            output,
            finished: false,
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
    pub fn add_track(&mut self, track: TrackParameters) -> Result<(), Error> {
        let track_id = track.track_id;
        if self.finished {
            return Err(Error::Finished);
        }
        if self
            .tracks
            .iter()
            .any(|added| added.first_decoding_time.is_some())
        {
            return Err(Error::TrackAfterMediaData { track_id });
        }
        check_track(self.tracks.iter().map(|added| &added.parameters), &track)?;
        let samples = SampleTableWriter::new(
            &TrackSampleDescription {
                track_id: track.track_id,
                stsd: &track.stsd,
                dref: &data_reference(),
            },
            self.config.sample_count_limit,
        );
        self.tracks.push(Track {
            parameters: track,
            samples,
            first_decoding_time: None,
        });
        Ok(())
    }

    /// Takes the next sample of a track, with its bytes, and holds them for [`poll_output`](Self::poll_output)
    ///
    /// The sample lies after the samples taken before it. Its decoding time
    /// counts from that of the first sample of its track, as the sample table
    /// declares it.
    ///
    /// # Errors
    /// * [`Error::Finished`]: the muxer has finished.
    /// * [`Error::BufferFull`]: the bytes would grow what the muxer holds past
    ///   [`MuxerConfig::buffer_limit`].
    /// * [`Error::UnknownTrack`]: no track of the sample's `track_ID` was added.
    /// * [`Error::StructureData`]: the sample table of the track refuses the
    ///   sample: it names a sample entry the track does not have, its decoding
    ///   time is not where the samples before it end, or the track holds as
    ///   many samples as [`MuxerConfig::sample_count_limit`].
    /// * [`Error::Overflow`]: the sample is longer than a sample table counts,
    ///   or would end past the last offset a file can have.
    pub fn handle_sample(
        &mut self,
        properties: SampleProperties,
        bytes: &[u8],
    ) -> Result<(), Error> {
        if self.finished {
            return Err(Error::Finished);
        }
        let length = self.output.len().saturating_add(bytes.len());
        if length > self.config.buffer_limit {
            return Err(Error::BufferFull { length });
        }
        let track_id = properties.track_id;
        let track = self
            .tracks
            .iter_mut()
            .find(|track| track.parameters.track_id == track_id)
            .ok_or(Error::UnknownTrack { track_id })?;
        let size = u32::try_from(bytes.len()).map_err(|_| Error::Overflow)?;
        let next = self
            .position
            .checked_add(u64::from(size))
            .ok_or(Error::Overflow)?;
        let first = track
            .first_decoding_time
            .unwrap_or(properties.decoding_time);
        // Why pass u64::MAX for a decoding time before the first: no sample
        // can lie there, so the sample table refuses it as a gap, which is
        // then reported on the caller's time line.
        let decoding_time = properties
            .decoding_time
            .checked_sub(first)
            .unwrap_or(u64::MAX);
        let location = SampleLocation {
            properties: SampleProperties {
                decoding_time,
                ..properties
            },
            offset: self.position,
            size,
        };
        track
            .samples
            .handle_location(location)
            .map_err(|error| {
                if let structure_data::Error::DecodingTimeGap {
                    track_id, expected, ..
                } = error
                {
                    structure_data::Error::DecodingTimeGap {
                        track_id,
                        expected: expected.saturating_add(first),
                        found: properties.decoding_time,
                    }
                } else {
                    error
                }
            })
            .map_err(Error::StructureData)?;
        track.first_decoding_time = Some(first);
        self.position = next;
        self.output.extend_from_slice(bytes);
        Ok(())
    }

    /// Appends the bytes the muxer holds to `out`, and holds none
    pub fn poll_output(&mut self, out: &mut Vec<u8>) {
        out.append(&mut self.output);
    }

    /// Builds the `moov` of the samples taken, holds it for [`poll_output`](Self::poll_output), and returns the header the `mdat` takes
    ///
    /// The `moov` follows the media data, whatever
    /// [`MuxerConfig::buffer_limit`] holds. The muxer then takes nothing more: it ends the file, unlike the
    /// `finish` of a writer of structure-data, which ends one unit and goes on.
    ///
    /// # Errors
    /// * [`Error::Finished`]: the muxer has finished.
    /// * [`Error::Encode`]: the `moov` does not write, as where a sample
    ///   entry or a box of the parameters does not.
    pub fn finish(&mut self) -> Result<MediaDataHeader, Error> {
        if self.finished {
            return Err(Error::Finished);
        }
        let payload_length = self.position.saturating_sub(
            self.media_data_offset
                .saturating_add(MEDIA_DATA_HEADER_LENGTH),
        );
        let header = MediaDataHeader {
            offset: self.media_data_offset,
            bytes: media_data_header(payload_length).ok_or(Error::Overflow)?,
        };
        let mut trak = Vec::with_capacity(self.tracks.len());
        for track in &self.tracks {
            // Why finish a clone: a `moov` that does not write leaves the
            // muxer as it was, samples and all.
            let stbl = track.samples.clone().finish();
            trak.push(track.parameters.to_track_box(stbl, self.movie.timescale));
        }
        append_encoded(&self.movie.to_movie_box(trak), &mut self.output)?;
        self.finished = true;
        Ok(header)
    }
}
