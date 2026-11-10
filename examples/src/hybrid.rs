//! [`HybridWriter`], which writes a movie that reads as fragmented while it is written and as a movie file once it ends, from the layers of `isobmff` alone

use isobmff::framing::media_data_header;
use isobmff::structure_data::{
    self, MovieFragmentWriter, SampleTableWriter, TrackSampleDescription,
};
use isobmff::{SampleLocation, SampleProperties};
use isobmff_boxes::{
    FileTypeBox, Free, FreeSpaceBox, MovieBox, MovieExtendsBox, TrackBox, TrackExtendsBox,
};
use isobmff_core::{BoxDefinition, BoxEncode};

/// Length of the header of an `mdat` as [`media_data_header`] writes it, which the `free` that opens the movie is as long as
const MEDIA_DATA_HEADER_LENGTH: u64 = 16;

/// Why a [`HybridWriter`] refuses a sample or a movie
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Error {
    /// A movie fragment or a sample table refuses a sample
    StructureData(structure_data::Error),
    /// A box does not write
    Encode(isobmff_core::Error),
    /// The file would grow past the last offset a file can have
    Overflow,
}

impl core::fmt::Display for Error {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::StructureData(_) => write!(formatter, "the structure-data refuses the sample"),
            Self::Encode(_) => write!(formatter, "a box does not write"),
            Self::Overflow => write!(
                formatter,
                "the file grows past the last offset there can be"
            ),
        }
    }
}

impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::StructureData(source) => Some(source),
            Self::Encode(source) => Some(source),
            Self::Overflow => None,
        }
    }
}

/// The header of the `mdat` a caller writes over the `free` that opens the movie, once the [`HybridWriter`] has finished
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MediaDataHeader {
    /// File offset of the `free`
    pub offset: u64,
    /// The header of an `mdat` that holds every byte from the end of the `free` to the `moov` that ends the file
    pub bytes: [u8; 16],
}

/// Writer of a movie of fragments that, once it ends, lie in one `mdat` that a `moov` after them declares
///
/// The movie is written in the order ISO/IEC 14496-12 lays a fragmented
/// movie file out (§8.8): an `ftyp`; a `free` (§8.1.2) as long as the header
/// of an `mdat`; a `moov` with an `mvex` and no sample; then each fragment, a
/// `moof` that [`MovieFragmentWriter`] writes and the `mdat` of its samples.
/// Until [`finish`](Self::finish), the bytes written read as a fragmented
/// movie file. `finish` appends a `moov` without an `mvex` whose sample
/// tables, which [`SampleTableWriter`] writes, declare every sample where its
/// fragment put it, and returns the header of an `mdat` that the caller
/// writes over the `free`. The file then reads as an `ftyp`, one `mdat` and a
/// `moov`: the first `moov` and the fragments lie inside the `mdat`.
///
/// The writer takes the tracks of a `moov` it is given, and keeps each of
/// them but for its sample table and durations. It uses no demuxer and no
/// muxer of `isobmff`.
#[derive(Debug, Clone)]
pub struct HybridWriter {
    movie: MovieBox,
    fragments: MovieFragmentWriter,
    tables: Vec<SampleTableWriter>,
    /// Decoding time of the first sample of each track, from which its sample
    /// table counts
    first_decoding_times: Vec<Option<u64>>,
    /// The samples of the fragment being written, each with the index of its
    /// track and at its offset in the media data of the fragment
    fragment: Vec<(usize, SampleLocation)>,
    media_data: Vec<u8>,
    /// File offset of the `free` that opens the movie
    free_offset: u64,
    /// File offset of the next byte written
    position: u64,
}

impl HybridWriter {
    /// Creates a writer of the tracks of `movie` in a file that opens with `file_type`, and appends the `ftyp`, the `free` and the `moov` of no sample to `out`
    ///
    /// # Errors
    /// * [`Error::Encode`]: a box does not write.
    /// * [`Error::Overflow`]: the boxes are longer than a file can be.
    pub fn new(file_type: &FileTypeBox, movie: MovieBox, out: &mut Vec<u8>) -> Result<Self, Error> {
        let tracks: Vec<TrackSampleDescription<'_>> = movie.trak.iter().map(description).collect();
        let fragments = MovieFragmentWriter::new(&tracks, u64::MAX);
        let tables: Vec<SampleTableWriter> = tracks
            .iter()
            .map(|track| SampleTableWriter::new(track, u64::MAX))
            .collect();
        let mut fragmented = movie.clone();
        let mut mvex = MovieExtendsBox::default();
        for (trak, table) in fragmented.trak.iter_mut().zip(&tables) {
            trak.mdia.minf.stbl = table.clone().finish();
            let mut trex = TrackExtendsBox::default();
            trex.track_id = trak.tkhd.track_id;
            trex.default_sample_description_index = 1;
            mvex.trex.push(trex);
        }
        fragmented.mvex = Some(mvex);

        let mut free = FreeSpaceBox::<Free>::default();
        free.data = vec![0; 8];
        let ftyp = encoded(file_type)?;
        let opening = [ftyp.as_slice(), &encoded(&free)?, &encoded(&fragmented)?].concat();
        let free_offset = length_of(out.len().saturating_add(ftyp.len()))?;
        out.extend_from_slice(&opening);
        Ok(Self {
            first_decoding_times: vec![None; movie.trak.len()],
            movie,
            fragments,
            tables,
            fragment: Vec::new(),
            media_data: Vec::new(),
            free_offset,
            position: length_of(out.len())?,
        })
    }

    /// Takes the next sample of the fragment being written, with its bytes
    ///
    /// # Errors
    /// * [`Error::StructureData`]: the movie fragment refuses the sample, as
    ///   where it is of a track the movie does not have.
    /// * [`Error::Overflow`]: the sample is longer than a fragment counts.
    pub fn handle_sample(
        &mut self,
        properties: SampleProperties,
        bytes: &[u8],
    ) -> Result<(), Error> {
        let track = self
            .movie
            .trak
            .iter()
            .position(|trak| trak.tkhd.track_id == properties.track_id);
        let location = SampleLocation {
            properties,
            offset: length_of(self.media_data.len())?,
            size: u32::try_from(bytes.len()).map_err(|_| Error::Overflow)?,
        };
        self.fragments
            .handle_location(location)
            .map_err(Error::StructureData)?;
        if let Some(track) = track {
            self.fragment.push((track, location));
        }
        self.media_data.extend_from_slice(bytes);
        Ok(())
    }

    /// Ends the fragment being written, and appends its `moof` and its `mdat` to `out`
    ///
    /// # Errors
    /// * [`Error::StructureData`]: a sample table refuses a sample of the
    ///   fragment.
    /// * [`Error::Encode`]: the `moof` does not write.
    /// * [`Error::Overflow`]: the file would grow past the last offset there
    ///   can be.
    pub fn finish_fragment(&mut self, out: &mut Vec<u8>) -> Result<(), Error> {
        if !self.fragments.has_samples() {
            return Ok(());
        }
        let moof = encoded(&self.fragments.finish())?;
        let media_data_length = length_of(self.media_data.len())?;
        let header = media_data_header(media_data_length).ok_or(Error::Overflow)?;
        let media_data_offset = self
            .position
            .checked_add(length_of(moof.len())?)
            .and_then(|offset| offset.checked_add(MEDIA_DATA_HEADER_LENGTH))
            .ok_or(Error::Overflow)?;
        for (track, location) in self.fragment.drain(..) {
            let properties = location.properties;
            let (Some(table), Some(first)) = (
                self.tables.get_mut(track),
                self.first_decoding_times.get_mut(track),
            ) else {
                continue;
            };
            let first = *first.get_or_insert(properties.decoding_time);
            table
                .handle_location(SampleLocation {
                    properties: SampleProperties {
                        decoding_time: properties.decoding_time.saturating_sub(first),
                        ..properties
                    },
                    offset: media_data_offset
                        .checked_add(location.offset)
                        .ok_or(Error::Overflow)?,
                    size: location.size,
                })
                .map_err(Error::StructureData)?;
        }
        out.extend_from_slice(&moof);
        out.extend_from_slice(&header);
        out.append(&mut self.media_data);
        self.position = media_data_offset
            .checked_add(media_data_length)
            .ok_or(Error::Overflow)?;
        Ok(())
    }

    /// Ends the last fragment, appends the `moov` that declares every sample to `out`, and returns the header the caller writes over the `free`
    ///
    /// # Errors
    /// As [`finish_fragment`](Self::finish_fragment), and [`Error::Encode`]
    /// where the `moov` does not write.
    pub fn finish(mut self, out: &mut Vec<u8>) -> Result<MediaDataHeader, Error> {
        self.finish_fragment(out)?;
        let mut moov = self.movie;
        let edits_run_to_the_end = moov.mvex.take().is_some_and(|mvex| mvex.mehd.is_none());
        for (trak, table) in moov.trak.iter_mut().zip(&mut self.tables) {
            trak.mdia.minf.stbl = table.finish();
        }
        declare_durations(&mut moov, edits_run_to_the_end);
        let payload_length = self
            .position
            .checked_sub(self.free_offset)
            .and_then(|length| length.checked_sub(MEDIA_DATA_HEADER_LENGTH))
            .ok_or(Error::Overflow)?;
        let bytes = media_data_header(payload_length).ok_or(Error::Overflow)?;
        out.extend_from_slice(&encoded(&moov)?);
        Ok(MediaDataHeader {
            offset: self.free_offset,
            bytes,
        })
    }
}

/// The sample entries and data references of `trak`
fn description(trak: &TrackBox) -> TrackSampleDescription<'_> {
    TrackSampleDescription {
        track_id: trak.tkhd.track_id,
        stsd: &trak.mdia.minf.stbl.stsd,
        dref: &trak.mdia.minf.dinf.dref,
    }
}

/// Sets the durations of the media of each track of `moov`, of the track and of the movie, to those its sample tables and edit lists give
///
/// Where `edits_run_to_the_end`, the `moov` was that of a fragmented movie
/// without a `mehd`, whose last edit of a duration of 0 runs to the end of the
/// media, §8.6.6.1; it is given the duration it runs for, as a movie file
/// takes it.
fn declare_durations(moov: &mut MovieBox, edits_run_to_the_end: bool) {
    let movie_timescale = u128::from(moov.mvhd.timescale);
    for trak in &mut moov.trak {
        let media_duration = trak.mdia.minf.stbl.stts.as_ref().map_or(0, |stts| {
            stts.entries.iter().fold(0_u64, |total, entry| {
                total.saturating_add(
                    u64::from(entry.sample_count).saturating_mul(u64::from(entry.sample_delta)),
                )
            })
        });
        trak.mdia.mdhd.duration = media_duration;
        trak.mdia.mdhd.version = u8::from(media_duration > u64::from(u32::MAX));
        let media_timescale = u128::from(trak.mdia.mdhd.timescale);
        if let Some(last) = trak
            .edts
            .as_mut()
            .and_then(|edts| edts.elst.as_mut())
            .and_then(|elst| elst.entries.last_mut())
            .filter(|last| edits_run_to_the_end && last.edit_duration == 0 && last.media_time >= 0)
        {
            let remaining = media_duration.saturating_sub(last.media_time.unsigned_abs());
            last.edit_duration = u128::from(remaining)
                .saturating_mul(movie_timescale)
                .checked_div(media_timescale)
                .and_then(|duration| u64::try_from(duration).ok())
                .unwrap_or(u64::MAX);
        }
        let edited = trak
            .edts
            .as_ref()
            .and_then(|edts| edts.elst.as_ref())
            .map(|elst| {
                elst.entries.iter().fold(0_u64, |total, entry| {
                    total.saturating_add(entry.edit_duration)
                })
            });
        let duration = edited.unwrap_or_else(|| {
            u128::from(media_duration)
                .saturating_mul(movie_timescale)
                .checked_div(media_timescale)
                .and_then(|duration| u64::try_from(duration).ok())
                .unwrap_or(u64::MAX)
        });
        trak.tkhd.duration = duration;
        trak.tkhd.version = u8::from(duration > u64::from(u32::MAX));
    }
    moov.mvhd.duration = moov
        .trak
        .iter()
        .map(|trak| trak.tkhd.duration)
        .max()
        .unwrap_or(0);
    moov.mvhd.version = u8::from(moov.mvhd.duration > u64::from(u32::MAX));
}

/// The whole box `value` forms
fn encoded(value: &(impl BoxEncode + BoxDefinition)) -> Result<Vec<u8>, Error> {
    let mut bytes = vec![0; usize::try_from(value.encoded_len()).map_err(|_| Error::Overflow)?];
    value.encode(&mut bytes).map_err(Error::Encode)?;
    Ok(bytes)
}

fn length_of(length: usize) -> Result<u64, Error> {
    u64::try_from(length).map_err(|_| Error::Overflow)
}
