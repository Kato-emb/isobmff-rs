//! A movie that reads as a fragmented movie file while it is written, and as a movie file once it ends, written from the layers of `isobmff` alone
//!
//! The example reads a file with the demuxer, then writes its samples again
//! with [`HybridWriter`], which uses no muxer: only the movie fragment writer
//! and the sample table writer of [`structure_data`], the box header of
//! [`framing`](isobmff::framing), and the boxes. The bytes written up to its `finish` are a
//! fragmented movie file; `finish` appends a `moov` that declares every
//! sample, and the `mdat` header it returns, written over the `free` that
//! opens the file, puts the fragments inside one `mdat`.
//!
//! `cargo run --example hybrid -- <input> <output> [samples_per_fragment]`

use isobmff::boxes::{
    FileTypeBox, Free, FreeSpaceBox, MovieBox, MovieExtendsBox, TrackBox, TrackExtendsBox,
};
use isobmff::demuxer::{self, Event};
use isobmff::framing::media_data_header;
use isobmff::structure_data::{
    self, MovieFragmentWriter, SampleTableWriter, TrackSampleDescription,
};
use isobmff::{BoxDefinition, BoxEncode, Demuxer, DemuxerConfig, SampleLocation, SampleProperties};

// Why not take the limit from the muxer configuration: the example writes
// without a muxer; this is the limit a muxer takes by default.
/// The most samples the writer takes of a fragment, and of a track
const SAMPLE_COUNT_LIMIT: u64 = 1 << 20;

/// What the demuxer reads of a file: its `ftyp`, its `moov`, and every sample of its tracks with its bytes, in file order
#[derive(Debug, Clone, PartialEq)]
struct Read {
    /// The `ftyp` of the file
    file_type: FileTypeBox,
    /// The `moov` of the file
    movie: MovieBox,
    /// What the structure-data declares of each sample, with its bytes
    samples: Vec<(SampleProperties, Vec<u8>)>,
    /// What the demuxer could not read
    errors: Vec<demuxer::Error>,
}

/// Reads `file` where the demuxer asks, subscribing to every track, or returns `None` where it holds no `ftyp` or no `moov`
#[must_use]
fn read(file: &[u8]) -> Option<Read> {
    let mut demuxer = Demuxer::new(DemuxerConfig::default());
    let (mut file_type, mut movie) = (None, None);
    let (mut samples, mut errors) = (Vec::new(), Vec::new());
    loop {
        while let Some(event) = demuxer.poll_event() {
            match event {
                Event::FileType(ftyp) => file_type = Some(ftyp),
                Event::Movie(moov) => {
                    for trak in &moov.trak {
                        demuxer.subscribe_track(trak.tkhd.track_id);
                    }
                    movie = Some(*moov);
                }
                Event::Sample(sample) => samples.push((sample.properties, sample.bytes.to_vec())),
                Event::Error(error) => errors.push(error),
                Event::SegmentType(_)
                | Event::MovieFragment(_)
                | Event::SampleLocation(_)
                | Event::Box { .. }
                | _ => {}
            }
        }
        let Some(required) = demuxer.required_input() else {
            break;
        };
        match usize::try_from(required.position)
            .ok()
            .and_then(|position| file.get(position..))
        {
            Some(bytes) if !bytes.is_empty() => demuxer.handle_input(required.position, bytes),
            _ => demuxer.finish(),
        }
    }
    Some(Read {
        file_type: file_type?,
        movie: movie?,
        samples,
        errors,
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let usage = "usage: hybrid <input> <output> [samples_per_fragment]";
    let (input, output) = (
        arguments.next().ok_or(usage)?,
        arguments.next().ok_or(usage)?,
    );
    let samples_per_fragment = arguments
        .next()
        .map_or(Ok(30), |count| count.parse::<usize>())?;
    let source = read(&std::fs::read(input)?).ok_or("no ftyp or no moov")?;
    let written = write_hybrid(&source, samples_per_fragment)?.1;
    std::fs::write(output, written)?;
    Ok(())
}

/// What a [`HybridWriter`] writes of the samples `source` holds, a fragment for every `samples_per_fragment` of them: the bytes before it finishes, and the file once it has
fn write_hybrid(source: &Read, samples_per_fragment: usize) -> Result<(Vec<u8>, Vec<u8>), Error> {
    let mut written = Vec::new();
    let mut writer = HybridWriter::new(
        &source.file_type,
        source.movie.clone(),
        SAMPLE_COUNT_LIMIT,
        &mut written,
    )?;
    for (index, (properties, bytes)) in source.samples.iter().enumerate() {
        if index > 0 && index.checked_rem(samples_per_fragment) == Some(0) {
            writer.finish_fragment(&mut written)?;
        }
        writer.handle_sample(*properties, bytes)?;
    }
    writer.finish_fragment(&mut written)?;
    let fragmented = written.clone();
    let header = writer.finish(&mut written)?;
    if let Some(free) = usize::try_from(header.offset)
        .ok()
        .and_then(|offset| written.get_mut(offset..))
        .and_then(|rest| rest.get_mut(..header.bytes.len()))
    {
        free.copy_from_slice(&header.bytes);
    }
    Ok((fragmented, written))
}

/// Length of the header of an `mdat` as [`media_data_header`] writes it, which the `free` that opens the movie is as long as
const MEDIA_DATA_HEADER_LENGTH: u64 = 16;

/// Why a [`HybridWriter`] refuses a sample or a movie
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Error {
    /// A movie fragment or a sample table refuses a sample
    StructureData(structure_data::Error),
    /// A box does not write
    Encode(isobmff::Error),
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
struct MediaDataHeader {
    /// File offset of the `free`
    offset: u64,
    /// The header of an `mdat` that holds every byte from the end of the `free` to the `moov` that ends the file
    bytes: [u8; 16],
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
struct HybridWriter {
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
    /// The writer takes at most `sample_count_limit` samples of a fragment,
    /// and of each track: it holds some tens of bytes of each until it ends.
    ///
    /// # Errors
    /// * [`Error::Encode`]: a box does not write.
    /// * [`Error::Overflow`]: the boxes are longer than a file can be.
    fn new(
        file_type: &FileTypeBox,
        movie: MovieBox,
        sample_count_limit: u64,
        out: &mut Vec<u8>,
    ) -> Result<Self, Error> {
        let tracks: Vec<TrackSampleDescription<'_>> = movie.trak.iter().map(description).collect();
        let fragments = MovieFragmentWriter::new(&tracks, sample_count_limit);
        let tables: Vec<SampleTableWriter> = tracks
            .iter()
            .map(|track| SampleTableWriter::new(track, sample_count_limit))
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
    ///   where it is of a track the movie does not have, or the fragment holds
    ///   as many samples as the limit.
    /// * [`Error::Overflow`]: the sample is longer than a fragment counts.
    fn handle_sample(&mut self, properties: SampleProperties, bytes: &[u8]) -> Result<(), Error> {
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
    ///   fragment, as where the track holds as many samples as the limit.
    /// * [`Error::Encode`]: the `moof` does not write.
    /// * [`Error::Overflow`]: the file would grow past the last offset there
    ///   can be.
    fn finish_fragment(&mut self, out: &mut Vec<u8>) -> Result<(), Error> {
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
    fn finish(mut self, out: &mut Vec<u8>) -> Result<MediaDataHeader, Error> {
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use isobmff::SampleProperties;
    use isobmff::framing::{BoxReader, Event, FramerConfig};
    use isobmff::structure_data;
    use isobmff_test_support::{conformance_files, ffprobe, ffprobe_packets};

    use super::{Error, read, write_hybrid};

    /// The fixtures of the reading tests, two movie files and two of movie fragments
    const FIXTURES: [&[u8]; 4] = [
        include_bytes!("../crates/isobmff/tests/fixtures/movie_moov_first.mp4"),
        include_bytes!("../crates/isobmff/tests/fixtures/movie_moov_last.mp4"),
        include_bytes!("../crates/isobmff/tests/fixtures/fragmented.mp4"),
        include_bytes!("../crates/isobmff/tests/fixtures/segment.mp4"),
    ];

    /// The samples of `source`, the decoding times of each track counted from that of its first sample, as a sample table declares them
    fn counted_from_the_first(
        samples: &[(SampleProperties, Vec<u8>)],
    ) -> Vec<(SampleProperties, Vec<u8>)> {
        let mut firsts = BTreeMap::new();
        samples
            .iter()
            .map(|(properties, bytes)| {
                let first = *firsts
                    .entry(properties.track_id)
                    .or_insert(properties.decoding_time);
                let properties = SampleProperties {
                    decoding_time: properties.decoding_time.checked_sub(first).unwrap(),
                    ..*properties
                };
                (properties, bytes.clone())
            })
            .collect()
    }

    /// The type of each box at file level of `file`
    fn top_level_box_types(file: &[u8]) -> Vec<String> {
        let mut config = FramerConfig::new(0);
        config.gathered_box_types.clear();
        let mut reader = BoxReader::new(config);
        let mut types = Vec::new();
        loop {
            while let Some(event) = reader.poll_event() {
                if let Event::Box { header, .. } = event {
                    types.push(header.box_type().to_string());
                }
            }
            let Some(required) = reader.required_input() else {
                break;
            };
            match file.get(usize::try_from(required.position).unwrap()..) {
                Some(bytes) if !bytes.is_empty() => reader.handle_input(required.position, bytes),
                _ => reader.finish(),
            }
        }
        types
    }

    #[test]
    fn a_movie_reads_as_fragmented_while_it_is_written_and_as_a_movie_file_once_it_ends() {
        for file in FIXTURES {
            let source = read(file).unwrap();
            let (fragmented, finished) = write_hybrid(&source, 16).unwrap();

            let while_written = read(&fragmented).unwrap();
            assert_eq!(
                (
                    while_written.samples,
                    while_written.movie.mvex.is_some(),
                    while_written.errors
                ),
                (source.samples.clone(), true, Vec::new())
            );

            let once_ended = read(&finished).unwrap();
            assert_eq!(
                (
                    once_ended.samples,
                    once_ended.movie.mvex.is_some(),
                    once_ended.errors,
                    top_level_box_types(&finished)
                ),
                (
                    counted_from_the_first(&source.samples),
                    false,
                    Vec::new(),
                    ["ftyp", "mdat", "moov"].map(String::from).to_vec()
                )
            );
        }
    }

    /// What ffprobe reads of each packet of `path`, ignoring edit lists: its track, its size, the offset of its composition time from its decoding time, and whether it is a key frame
    fn packets(path: &Path) -> Vec<(u32, u64, Option<i64>, bool)> {
        ffprobe_packets(&["-ignore_editlist", "1"], path)
            .iter()
            .map(|packet| {
                (
                    packet.track_id,
                    packet.size,
                    packet
                        .pts
                        .zip(packet.dts)
                        .map(|(pts, dts)| pts.checked_sub(dts).unwrap()),
                    packet.key,
                )
            })
            .collect()
    }

    #[test]
    #[ignore = "needs ISOBMFF_CONFORMANCE_DIR and ffprobe"]
    fn the_conformance_files_written_as_hybrid_movies_read_as_the_same_samples_and_packets() {
        let directory = std::env::temp_dir().join(format!("isobmff-hybrid-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let (mut differences, mut written_files, mut compared, mut refused) = (Vec::new(), 0, 0, 0);
        for file in conformance_files() {
            let Some(source) = read(&file.bytes) else {
                continue;
            };
            // Why skip these: a file the demuxer reports an error in, such as
            // one whose samples lie in another file, has none to write.
            if !source.errors.is_empty() || source.samples.is_empty() {
                continue;
            }
            let (fragmented, finished) = match write_hybrid(&source, 30) {
                Ok(written) => written,
                // Why accept this refusal: a `tfdt` is the sum of the
                // durations of the samples before it, §8.8.12, and the source
                // jumps.
                Err(Error::StructureData(structure_data::Error::DecodingTimeGap { .. })) => {
                    refused += 1;
                    continue;
                }
                Err(error) => {
                    differences.push(format!("{}: {error:?}", file.name));
                    continue;
                }
            };
            written_files += 1;
            let while_written = read(&fragmented).unwrap();
            let once_ended = read(&finished).unwrap();
            if (&while_written.samples, &while_written.errors) != (&source.samples, &Vec::new()) {
                differences.push(format!(
                    "{}: the samples read while written differ",
                    file.name
                ));
            }
            if (&once_ended.samples, &once_ended.errors)
                != (&counted_from_the_first(&source.samples), &Vec::new())
            {
                differences.push(format!("{}: the samples read once ended differ", file.name));
            }
            // Why compare these with this crate's demuxer alone: ffprobe
            // refuses to read the source.
            if !ffprobe(&["-v", "error"], &file.path).status.success() {
                continue;
            }
            let presented = |path: &Path| ffprobe_packets(&[], path).len();
            let written_path = directory.join(format!("presented-{}", file.name));
            std::fs::write(&written_path, &finished).unwrap();
            if presented(&written_path) != presented(&file.path) {
                differences.push(format!(
                    "{} once ended: ffprobe presents {} packets through the edit lists, {} of the source",
                    file.name,
                    presented(&written_path),
                    presented(&file.path)
                ));
            }
            let theirs = packets(&file.path);
            for (stage, bytes) in [("while written", &fragmented), ("once ended", &finished)] {
                let path = directory.join(format!("{stage}-{}", file.name));
                std::fs::write(&path, bytes).unwrap();
                let ours = packets(&path);
                compared += 1;
                if ours != theirs {
                    differences.push(format!(
                        "{} {stage}: ffprobe reads {} packets, {} from the source; first differing {:?}",
                        file.name,
                        ours.len(),
                        theirs.len(),
                        ours.iter().zip(&theirs).find(|(ours, theirs)| ours != theirs)
                    ));
                }
            }
        }
        std::fs::remove_dir_all(&directory).unwrap();
        println!(
            "{written_files} files written, {compared} compared with ffprobe, {refused} refused for a gap in decoding times"
        );
        assert_eq!(differences, Vec::<String>::new());
        assert!(written_files > 0);
    }
}
