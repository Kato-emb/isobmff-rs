//! [`Muxer`], which writes the structure of a movie file around the samples a caller writes
//!
//! A caller describes the movie and its tracks with [`MovieDescription`] and
//! [`TrackDescription`], hands the [`Muxer`] what it declares of each sample,
//! and writes the bytes the muxer returns and the bytes of each sample where
//! the muxer places it.

use alloc::vec;
use alloc::vec::Vec;
use core::num::NonZeroU128;

use isobmff_boxes::{
    DataEntry, DataEntryUrlBox, EditBox, FileTypeBox, HandlerBox, HintMediaHeaderBox,
    MediaInformationHeader, MovieBox, NullMediaHeaderBox, SampleTableBox, SoundMediaHeaderBox,
    SubtitleMediaHeaderBox, TrackBox, VideoMediaHeaderBox,
};
use isobmff_core::{BoxDefinition, BoxEncode, I8F8};

use crate::framing;
use crate::structure_data::{self, SampleTableWriter};
use crate::{SampleLocation, SampleProperties};

mod config;
mod description;
mod error;

pub use config::MuxerConfig;
pub use description::{MovieDescription, TrackDescription};
pub use error::Error;

/// How a [`Muxer`] lays the movie out in the file
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FileStructure {
    /// An `ftyp`, one `mdat` that holds every sample, and the `moov` that
    /// declares them, ISO/IEC 14496-12 §4.3, §8.1.1 and §8.2.1
    NonFragmented {
        /// The `ftyp` the file opens with, §4.3
        file_type: FileTypeBox,
    },
}

/// The header of the `mdat` a caller writes over the one it wrote first, once a [`Muxer`] has finished
///
/// The header of the `media data` box, ISO/IEC 14496-12 §8.1.1, as
/// [`framing::media_data_header`] writes it, with the length of every byte
/// the caller wrote in the box.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MediaDataHeader {
    /// File offset of the header
    pub offset: u64,
    /// The bytes of the header, of the length of the header first written
    pub bytes: [u8; 16],
}

/// Length of the header of the `mdat`, which [`framing::media_data_header`] writes with a `largesize`
const MEDIA_DATA_HEADER_LENGTH: u64 = 16;

/// A track added, and what the muxer has taken of its samples
#[derive(Debug, Clone)]
struct Track {
    description: TrackDescription,
    samples: SampleTableWriter,
    /// Decoding time of the first sample taken, from which the sample table
    /// counts
    first_decoding_time: Option<u64>,
}

/// Writer of the structure of a movie file around the samples a caller writes: the `ftyp`, the header of the `mdat` and the `moov`
///
/// The structure of a file, ISO/IEC 14496-12 §4.2.2, §8.1.1, §8.2 and §8.5.
///
/// The muxer takes values and returns bytes: [`add_track`](Self::add_track)
/// takes the description of each track, and
/// [`handle_sample`](Self::handle_sample) what the structure-data declares of
/// each sample, in decoding order within its track, and returns where the
/// caller writes its bytes. [`advance_position`](Self::advance_position) takes
/// the length of other bytes the caller writes into the media data.
/// [`finish`](Self::finish) builds the `moov`. [`poll_output`](Self::poll_output)
/// appends what the muxer holds: the `ftyp` and the header of the `mdat`
/// first, the `moov` once finished. A call that fails changes nothing.
///
/// The muxer writes no byte of a sample, and decides neither which samples
/// nor in what order across tracks: the caller writes the bytes of each
/// sample where `handle_sample` places it. The length of the `mdat` is known
/// only once every sample is in it, so the caller writes the header `finish`
/// returns over the one first written; a caller that cannot seek back writes
/// a fragmented file instead.
///
/// A movie file written again:
///
/// ```
/// use isobmff::muxer::{FileStructure, MovieDescription, TrackDescription};
/// use isobmff::{Demuxer, DemuxerConfig, Event, Muxer, MuxerConfig};
///
/// // Read the `ftyp`, the `moov` and every sample of a file
/// let file: &[u8] = include_bytes!("../tests/fixtures/movie_moov_first.mp4");
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
/// let structure = FileStructure::NonFragmented { file_type };
/// let mut muxer = Muxer::new(MovieDescription::from(&*movie), MuxerConfig::default(), structure)?;
/// for trak in &movie.trak {
///     muxer.add_track(TrackDescription::from(trak))?;
/// }
/// let mut written = Vec::new();
/// for (properties, bytes) in &samples {
///     let location = muxer.handle_sample(*properties, u32::try_from(bytes.len())?)?;
///     muxer.poll_output(&mut written);
///     assert_eq!(location.offset, u64::try_from(written.len())?);
///     written.extend_from_slice(bytes);
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
pub struct Muxer {
    movie: MovieDescription,
    tracks: Vec<Track>,
    sample_count_limit: u64,
    /// File offset of the header of the `mdat`
    media_data_offset: u64,
    /// File offset of the next byte the caller writes into the `mdat`
    position: u64,
    /// Bytes [`poll_output`](Self::poll_output) has yet to append
    output: Vec<u8>,
    finished: bool,
}

impl Muxer {
    /// Creates a muxer of the movie `movie` describes, laid out as `structure` says, and holds the bytes that open the file
    ///
    /// # Errors
    /// * [`Error::Encode`]: the `ftyp` does not write.
    pub fn new(
        movie: MovieDescription,
        config: MuxerConfig,
        structure: FileStructure,
    ) -> Result<Self, Error> {
        let FileStructure::NonFragmented { file_type } = structure;
        let mut output = Vec::new();
        append_encoded(&file_type, &mut output)?;
        let media_data_offset = u64::try_from(output.len()).map_err(|_| Error::Overflow)?;
        let header = framing::media_data_header(0).ok_or(Error::Overflow)?;
        output.extend_from_slice(&header);
        Ok(Self {
            movie,
            tracks: Vec::new(),
            sample_count_limit: config.sample_count_limit,
            media_data_offset,
            position: media_data_offset.saturating_add(MEDIA_DATA_HEADER_LENGTH),
            output,
            finished: false,
        })
    }

    /// Adds the track `track` describes, before the media data begins
    ///
    /// # Errors
    /// * [`Error::Finished`]: the muxer has finished.
    /// * [`Error::TrackAfterMediaData`]: a sample, or other bytes of the media
    ///   data, were taken.
    /// * [`Error::TrackId`]: the `track_ID` is 0 or that of a track added.
    /// * [`Error::DataReferenceIndex`]: a sample entry names another data
    ///   reference than 1, the media data in the same file.
    pub fn add_track(&mut self, track: TrackDescription) -> Result<(), Error> {
        let track_id = track.track_id;
        if self.finished {
            return Err(Error::Finished);
        }
        if self.position
            > self
                .media_data_offset
                .saturating_add(MEDIA_DATA_HEADER_LENGTH)
        {
            return Err(Error::TrackAfterMediaData { track_id });
        }
        if track_id == 0
            || self
                .tracks
                .iter()
                .any(|added| added.description.track_id == track_id)
        {
            return Err(Error::TrackId { track_id });
        }
        if let Some(data_reference_index) = track
            .sample_entries
            .sample_descriptions
            .iter()
            .map(|entry| entry.data_reference_index().unwrap_or(0))
            .find(|&index| index != 1)
        {
            return Err(Error::DataReferenceIndex {
                track_id,
                data_reference_index,
            });
        }
        self.tracks.push(Track {
            description: track,
            samples: SampleTableWriter::new(self.sample_count_limit),
            first_decoding_time: None,
        });
        Ok(())
    }

    /// Takes the next sample of a track, of `size` bytes, and returns where the caller writes them
    ///
    /// The sample lies at the end of the media data so far. Its decoding time
    /// counts from that of the first sample of its track, as the sample table
    /// declares it, and the location returned holds it so.
    ///
    /// # Errors
    /// * [`Error::Finished`]: the muxer has finished.
    /// * [`Error::UnknownTrack`]: no track of the sample's `track_ID` was added.
    /// * [`Error::StructureData`]: the sample table of the track refuses the
    ///   sample: it names a sample entry the track does not have, its decoding
    ///   time is not where the samples before it end, or the track holds as
    ///   many samples as [`MuxerConfig::sample_count_limit`].
    /// * [`Error::Overflow`]: the sample would end past the last offset a
    ///   file can have.
    pub fn handle_sample(
        &mut self,
        properties: SampleProperties,
        size: u32,
    ) -> Result<SampleLocation, Error> {
        if self.finished {
            return Err(Error::Finished);
        }
        let track_id = properties.track_id;
        let track = self
            .tracks
            .iter_mut()
            .find(|track| track.description.track_id == track_id)
            .ok_or(Error::UnknownTrack { track_id })?;
        // Why check here as well as in the sample table: a sample the table
        // would refuse only at `finish` is refused while the caller holds it.
        let sample_description_index = properties.sample_description_index;
        let entry_index = usize::try_from(sample_description_index)
            .ok()
            .and_then(|index| index.checked_sub(1));
        if entry_index
            .and_then(|index| {
                track
                    .description
                    .sample_entries
                    .sample_descriptions
                    .get(index)
            })
            .is_none()
        {
            return Err(Error::StructureData(
                structure_data::Error::SampleDescriptionIndexOutOfRange {
                    sample_description_index,
                },
            ));
        }
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
        Ok(location)
    }

    /// Takes `length` bytes the caller writes into the media data, after the samples so far
    ///
    /// # Errors
    /// * [`Error::Finished`]: the muxer has finished.
    /// * [`Error::Overflow`]: the bytes would end past the last offset a file
    ///   can have.
    pub fn advance_position(&mut self, length: u64) -> Result<(), Error> {
        if self.finished {
            return Err(Error::Finished);
        }
        self.position = self.position.checked_add(length).ok_or(Error::Overflow)?;
        Ok(())
    }

    /// Appends the bytes the muxer holds to `out`, and holds none
    pub fn poll_output(&mut self, out: &mut Vec<u8>) {
        out.append(&mut self.output);
    }

    /// Builds the `moov` of the samples taken, holds it for [`poll_output`](Self::poll_output), and returns the header the `mdat` takes
    ///
    /// The `moov` follows the media data. The muxer then takes nothing more.
    ///
    /// # Errors
    /// * [`Error::Finished`]: the muxer has finished.
    /// * [`Error::Encode`]: the `moov` does not write, as where a sample
    ///   entry or a box of a description does not.
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
            bytes: framing::media_data_header(payload_length).ok_or(Error::Overflow)?,
        };
        let mut moov = MovieBox::default();
        moov.mvhd.timescale = self.movie.timescale;
        for track in &self.tracks {
            moov.trak.push(track.to_box(self.movie.timescale)?);
        }
        moov.mvhd.duration = moov
            .trak
            .iter()
            .map(|trak| trak.tkhd.duration)
            .max()
            .unwrap_or(0);
        moov.mvhd.version = u8::from(moov.mvhd.duration > u64::from(u32::MAX));
        moov.mvhd.next_track_id = moov
            .trak
            .iter()
            .map(|trak| trak.tkhd.track_id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        moov.other_boxes = self.movie.other_boxes.clone();
        append_encoded(&moov, &mut self.output)?;
        self.finished = true;
        Ok(header)
    }
}

impl Track {
    /// The `trak` of the track, its durations in `movie_timescale` where the movie counts them
    fn to_box(&self, movie_timescale: u32) -> Result<TrackBox, Error> {
        let description = &self.description;
        // Why finish a clone: a `moov` that does not write leaves the muxer
        // as it was, samples and all.
        let stbl: SampleTableBox = self
            .samples
            .clone()
            .finish(&description.sample_entries)
            .map_err(Error::StructureData)?;
        let media_duration = stbl.stts.as_ref().map_or(0, |stts| {
            stts.entries.iter().fold(0_u64, |total, entry| {
                total.saturating_add(
                    u64::from(entry.sample_count).saturating_mul(u64::from(entry.sample_delta)),
                )
            })
        });
        let duration = match &description.edit_list {
            Some(elst) => elst.entries.iter().fold(0_u64, |total, entry| {
                total.saturating_add(entry.edit_duration)
            }),
            // Why round up: the track then lasts at least as long as its
            // media, as FFmpeg's movenc rounds it.
            None => NonZeroU128::new(u128::from(description.timescale)).map_or(0, |timescale| {
                let scaled = u128::from(media_duration)
                    .saturating_mul(u128::from(movie_timescale))
                    .div_ceil(timescale.get());
                u64::try_from(scaled).unwrap_or(u64::MAX)
            }),
        };
        let is_wide = |value: u64| u8::from(value > u64::from(u32::MAX));

        let mut trak = TrackBox::default();
        trak.tkhd.version = is_wide(duration);
        trak.tkhd.track_enabled = true;
        trak.tkhd.track_in_movie = true;
        trak.tkhd.track_in_preview = true;
        trak.tkhd.track_id = description.track_id;
        trak.tkhd.duration = duration;
        trak.tkhd.volume = if description.handler_type.as_bytes() == b"soun" {
            I8F8::ONE
        } else {
            I8F8::ZERO
        };
        trak.tkhd.width = description.width;
        trak.tkhd.height = description.height;
        trak.edts = description.edit_list.clone().map(|elst| {
            let mut edts = EditBox::default();
            edts.elst = Some(elst);
            edts
        });
        trak.mdia.mdhd.version = is_wide(media_duration);
        trak.mdia.mdhd.timescale = description.timescale;
        trak.mdia.mdhd.duration = media_duration;
        let mut hdlr = HandlerBox::default();
        hdlr.handler_type = description.handler_type;
        trak.mdia.hdlr = hdlr;
        trak.mdia.minf.media_information_header = Some(match description.handler_type.as_bytes() {
            b"vide" => MediaInformationHeader::Video(VideoMediaHeaderBox::default()),
            b"soun" => MediaInformationHeader::Sound(SoundMediaHeaderBox::default()),
            b"hint" => MediaInformationHeader::Hint(HintMediaHeaderBox::default()),
            b"subt" => MediaInformationHeader::Subtitle(SubtitleMediaHeaderBox::default()),
            _ => MediaInformationHeader::Null(NullMediaHeaderBox::default()),
        });
        trak.mdia.minf.dinf.dref.data_entries = vec![DataEntry::Url(DataEntryUrlBox::default())];
        trak.mdia.minf.stbl = stbl;
        trak.other_boxes = description.other_boxes.clone();
        Ok(trak)
    }
}

/// Appends the whole box `value` forms to `out`, leaving `out` as it was where it does not write
fn append_encoded(
    value: &(impl BoxEncode + BoxDefinition),
    out: &mut Vec<u8>,
) -> Result<(), Error> {
    let start = out.len();
    let length = usize::try_from(value.encoded_len()).map_err(|_| Error::Overflow)?;
    out.resize(start.saturating_add(length), 0);
    match value.encode(out.get_mut(start..).unwrap_or_default()) {
        Ok(_) => Ok(()),
        Err(source) => {
            out.truncate(start);
            Err(Error::Encode(source))
        }
    }
}
