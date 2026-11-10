//! A movie that reads as a fragmented movie file while it is written, and as a movie file once it ends, written from the layers of `isobmff` alone
//!
//! The example reads a file with the demuxer where it asks, and writes each
//! sample it lends without a muxer, with only the movie fragment writer and
//! the sample table writer of [`structure_data`], the `mdat` header of
//! [`framing`](isobmff::framing), and the boxes:
//!
//! 1. an `ftyp`, a `free` (§8.1.2) as long as the header of an `mdat`, and a
//!    `moov` with an `mvex` and no sample;
//! 2. each fragment, a `moof` and the `mdat` of its samples; up to here the
//!    file is a fragmented movie file (§8.8);
//! 3. a `moov` without an `mvex`, whose sample tables declare every sample
//!    where its fragment put it, and over the `free` the header of an `mdat`
//!    that holds the first `moov` and the fragments: the file is then an
//!    `ftyp`, one `mdat` and a `moov`.
//!
//! Memory is bounded by the limits of [`DemuxerConfig`], the bytes of one
//! fragment and some tens of bytes for each sample of the sample tables,
//! whatever the length of the file.
//!
//! `cargo run --example hybrid -- <input> <output> [samples_per_fragment]`

use std::error::Error;
use std::io::{Read, Seek, SeekFrom, Write};

use isobmff::boxes::{Free, FreeSpaceBox, MovieBox, MovieExtendsBox, TrackExtendsBox};
use isobmff::demuxer::Event;
use isobmff::framing::media_data_header;
use isobmff::structure_data::{MovieFragmentWriter, SampleTableWriter, TrackSampleDescription};
use isobmff::{BoxDefinition, BoxEncode, Demuxer, DemuxerConfig, SampleLocation, SampleProperties};

// Why not take the limit from the muxer configuration: the example writes
// without a muxer; this is the limit a muxer takes by default.
/// The most samples the writers take of a fragment, and of a track
const SAMPLE_COUNT_LIMIT: u64 = 1 << 20;

/// Length of the header of an `mdat` as [`media_data_header`] writes it, which the `free` that opens the movie is as long as
const MEDIA_DATA_HEADER_LENGTH: usize = 16;

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args().skip(1);
    let usage = "usage: hybrid <input> <output> [samples_per_fragment]";
    let (input, output) = (
        arguments.next().ok_or(usage)?,
        arguments.next().ok_or(usage)?,
    );
    let samples_per_fragment = arguments
        .next()
        .map_or(Ok(30), |count| count.parse::<usize>())?;
    hybrid(
        std::fs::File::open(input)?,
        std::fs::File::create(output)?,
        samples_per_fragment,
    )
}

/// Writes the samples of `input` to `output`, a fragment for every `samples_per_fragment` of them, then the `moov` that declares every sample, and the `mdat` header over the `free`
fn hybrid(
    mut input: impl Read + Seek,
    mut output: impl Write + Seek,
    samples_per_fragment: usize,
) -> Result<(), Box<dyn Error>> {
    let mut demuxer = Demuxer::new(DemuxerConfig::default());
    let (mut buffer, mut out) = (vec![0; 64 * 1024], Vec::new());
    let mut file_type = None;
    // Set once the `moov` is read: the movie, the writers of its fragments
    // and of the sample table of each track, and where the `free` lies
    let mut movie: Option<(MovieBox, MovieFragmentWriter, Vec<SampleTableWriter>, u64)> = None;
    let mut first_decoding_times = Vec::new();
    // The samples of the fragment being written, each with the index of its
    // track and at its offset in the media data of the fragment, and its bytes
    let (mut placed, mut media_data) = (Vec::new(), Vec::new());
    loop {
        while let Some(event) = demuxer.poll_event() {
            match event {
                Event::FileType(ftyp) => file_type = Some(ftyp),
                // The `ftyp`, the `free` to write the `mdat` header over, and the `moov` of no sample
                Event::Movie(moov) => {
                    append(&file_type.take().ok_or("a moov before any ftyp")?, &mut out)?;
                    let free_offset = u64::try_from(out.len())?;
                    let mut free = FreeSpaceBox::<Free>::default();
                    free.data = vec![0; MEDIA_DATA_HEADER_LENGTH - 8];
                    append(&free, &mut out)?;
                    let tracks: Vec<TrackSampleDescription<'_>> = moov
                        .trak
                        .iter()
                        .map(|trak| TrackSampleDescription {
                            track_id: trak.tkhd.track_id,
                            stsd: &trak.mdia.minf.stbl.stsd,
                            dref: &trak.mdia.minf.dinf.dref,
                        })
                        .collect();
                    append(&fragmented_movie_box(&moov, &tracks), &mut out)?;
                    output.write_all(&out)?;
                    out.clear();
                    let fragments = MovieFragmentWriter::new(&tracks, SAMPLE_COUNT_LIMIT);
                    let tables = tracks
                        .iter()
                        .map(|track| SampleTableWriter::new(track, SAMPLE_COUNT_LIMIT))
                        .collect();
                    first_decoding_times = vec![None; tracks.len()];
                    for trak in &moov.trak {
                        demuxer.subscribe_track(trak.tkhd.track_id);
                    }
                    movie = Some((*moov, fragments, tables, free_offset));
                }
                // Each sample joins the fragment being written, which is written once full
                Event::Sample(sample) => {
                    let (moov, fragments, tables, _) =
                        movie.as_mut().ok_or("a sample before any moov")?;
                    if placed.len() == samples_per_fragment.max(1) {
                        write_fragment(
                            fragments,
                            tables,
                            &mut first_decoding_times,
                            &mut placed,
                            &mut media_data,
                            &mut output,
                        )?;
                    }
                    let location = SampleLocation {
                        properties: sample.properties,
                        offset: u64::try_from(media_data.len())?,
                        size: u32::try_from(sample.bytes.len())?,
                    };
                    fragments.handle_location(location)?;
                    let track = moov
                        .trak
                        .iter()
                        .position(|trak| trak.tkhd.track_id == location.properties.track_id)
                        .ok_or("a sample of a track the moov does not have")?;
                    placed.push((track, location));
                    media_data.extend_from_slice(sample.bytes);
                }
                Event::Error(error) => return Err(error.into()),
                _ => {}
            }
        }
        let Some(required) = demuxer.required_input() else {
            break;
        };
        input.seek(SeekFrom::Start(required.position))?;
        match input.read(&mut buffer)? {
            0 => demuxer.finish(),
            read => demuxer.handle_input(required.position, &buffer[..read]),
        }
    }
    let (mut moov, mut fragments, mut tables, free_offset) = movie.ok_or("no moov")?;
    write_fragment(
        &mut fragments,
        &mut tables,
        &mut first_decoding_times,
        &mut placed,
        &mut media_data,
        &mut output,
    )?;

    // The `moov` that declares every sample, and the `mdat` header over the `free`
    let edits_run_to_the_end = moov.mvex.take().is_some_and(|mvex| mvex.mehd.is_none());
    for (trak, table) in moov.trak.iter_mut().zip(&mut tables) {
        trak.mdia.minf.stbl = table.finish();
    }
    declare_durations(&mut moov, edits_run_to_the_end);
    let end = output.stream_position()?;
    let header = media_data_header(end - free_offset - u64::try_from(MEDIA_DATA_HEADER_LENGTH)?)
        .ok_or("a file too long")?;
    append(&moov, &mut out)?;
    output.write_all(&out)?;
    output.seek(SeekFrom::Start(free_offset))?;
    output.write_all(&header)?;
    Ok(())
}

/// Writes the fragment of the samples `placed`, whose bytes are `media_data`, to `output`: its `moof`, the header of its `mdat`, its samples; and hands each sample, at the file offset it now lies at, to the sample table of its track
fn write_fragment(
    fragments: &mut MovieFragmentWriter,
    tables: &mut [SampleTableWriter],
    first_decoding_times: &mut [Option<u64>],
    placed: &mut Vec<(usize, SampleLocation)>,
    media_data: &mut Vec<u8>,
    output: &mut (impl Write + Seek),
) -> Result<(), Box<dyn Error>> {
    if !fragments.has_samples() {
        return Ok(());
    }
    let mut out = Vec::new();
    append(&fragments.finish(), &mut out)?;
    out.extend_from_slice(
        &media_data_header(u64::try_from(media_data.len())?).ok_or("a fragment too long")?,
    );
    output.write_all(&out)?;
    let media_data_offset = output.stream_position()?;
    output.write_all(media_data)?;
    for (track, location) in placed.drain(..) {
        let properties = location.properties;
        let first = *first_decoding_times[track].get_or_insert(properties.decoding_time);
        tables[track].handle_location(SampleLocation {
            properties: SampleProperties {
                decoding_time: properties.decoding_time - first,
                ..properties
            },
            offset: media_data_offset + location.offset,
            size: location.size,
        })?;
    }
    media_data.clear();
    Ok(())
}

/// The `moov` of the tracks of `movie`, described by `tracks`, which declares no sample and extends each track into movie fragments
fn fragmented_movie_box(movie: &MovieBox, tracks: &[TrackSampleDescription<'_>]) -> MovieBox {
    let mut moov = movie.clone();
    let mut mvex = MovieExtendsBox::default();
    for (trak, track) in moov.trak.iter_mut().zip(tracks) {
        trak.mdia.minf.stbl = SampleTableWriter::new(track, 0).finish();
        let mut trex = TrackExtendsBox::default();
        trex.track_id = track.track_id;
        trex.default_sample_description_index = 1;
        mvex.trex.push(trex);
    }
    moov.mvex = Some(mvex);
    moov
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

// Why not encode into a new `Vec` and copy it: the box is encoded in place at
// the end of what is held for writing.
/// Appends the whole box `value` forms to `out`
fn append(
    value: &(impl BoxEncode + BoxDefinition),
    out: &mut Vec<u8>,
) -> Result<(), Box<dyn Error>> {
    let start = out.len();
    out.resize(start + usize::try_from(value.encoded_len())?, 0);
    value.encode(&mut out[start..])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::error::Error;
    use std::io::Cursor;
    use std::path::Path;

    use isobmff::boxes::MovieBox;
    use isobmff::framing::{BoxReader, Event, FramerConfig};
    use isobmff::{Demuxer, DemuxerConfig, SampleProperties, demuxer, structure_data};
    use isobmff_test_support::{conformance_files, ffprobe, ffprobe_packets};

    use super::{MEDIA_DATA_HEADER_LENGTH, hybrid};

    /// What the structure-data declares of each sample of a file, with its bytes, in file order
    type Samples = Vec<(SampleProperties, Vec<u8>)>;

    /// The `moov` of `file` and every sample with its bytes, in file order, as the demuxer lends them, or `None` where it reports what it cannot read
    fn read(file: &[u8]) -> Option<(MovieBox, Samples)> {
        let mut demuxer = Demuxer::new(DemuxerConfig::default());
        let (mut movie, mut samples, mut readable) = (None, Vec::new(), true);
        loop {
            while let Some(event) = demuxer.poll_event() {
                match event {
                    demuxer::Event::Movie(moov) => {
                        for trak in &moov.trak {
                            demuxer.subscribe_track(trak.tkhd.track_id);
                        }
                        movie = Some(*moov);
                    }
                    demuxer::Event::Sample(sample) => {
                        samples.push((sample.properties, sample.bytes.to_vec()));
                    }
                    demuxer::Event::Error(_) => readable = false,
                    _ => {}
                }
            }
            let Some(required) = demuxer.required_input() else {
                break;
            };
            match file.get(usize::try_from(required.position).unwrap()..) {
                Some(bytes) if !bytes.is_empty() => demuxer.handle_input(required.position, bytes),
                _ => demuxer.finish(),
            }
        }
        readable.then_some((movie?, samples))
    }

    /// What the example writes of `file`, a fragment for every `samples_per_fragment` samples: the file as it stood before its last `moov` and its `mdat` header were written, and the file once they were
    fn write_hybrid(
        file: &[u8],
        samples_per_fragment: usize,
    ) -> Result<(Vec<u8>, Vec<u8>), Box<dyn Error>> {
        let mut output = Cursor::new(Vec::new());
        hybrid(Cursor::new(file), &mut output, samples_per_fragment)?;
        let finished = output.into_inner();
        let (_, last_moov) = top_level_boxes(&finished).pop().unwrap();
        let free = usize::try_from(u32::from_be_bytes(finished[..4].try_into().unwrap())).unwrap();
        let mut fragmented = finished[..usize::try_from(last_moov).unwrap()].to_vec();
        fragmented[free..free + MEDIA_DATA_HEADER_LENGTH]
            .copy_from_slice(b"\0\0\0\x10free\0\0\0\0\0\0\0\0");
        Ok((fragmented, finished))
    }

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
                    decoding_time: properties.decoding_time - first,
                    ..*properties
                };
                (properties, bytes.clone())
            })
            .collect()
    }

    /// The type of each box at file level of `file`
    fn top_level_box_types(file: &[u8]) -> Vec<String> {
        top_level_boxes(file)
            .into_iter()
            .map(|(box_type, _)| box_type)
            .collect()
    }

    /// The type of each box at file level of `file`, and its file offset
    fn top_level_boxes(file: &[u8]) -> Vec<(String, u64)> {
        let mut config = FramerConfig::new(0);
        config.gathered_box_types.clear();
        let mut reader = BoxReader::new(config);
        let mut types = Vec::new();
        loop {
            while let Some(event) = reader.poll_event() {
                if let Event::Box { offset, header } = event {
                    types.push((header.box_type().to_string(), offset));
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
            let (_, samples) = read(file).unwrap();
            let (fragmented, finished) = write_hybrid(file, 16).unwrap();

            let (movie_while_written, while_written) = read(&fragmented).unwrap();
            assert_eq!(
                (while_written, movie_while_written.mvex.is_some()),
                (samples.clone(), true)
            );

            let (movie_once_ended, once_ended) = read(&finished).unwrap();
            assert_eq!(
                (
                    once_ended,
                    movie_once_ended.mvex.is_some(),
                    top_level_box_types(&finished)
                ),
                (
                    counted_from_the_first(&samples),
                    false,
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
                    packet.pts.zip(packet.dts).map(|(pts, dts)| pts - dts),
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
            // Why skip these: a file the demuxer reports an error in, such as
            // one whose samples lie in another file, has none to write.
            let Some((_, samples)) = read(&file.bytes) else {
                continue;
            };
            if samples.is_empty() {
                continue;
            }
            let (fragmented, finished) = match write_hybrid(&file.bytes, 30) {
                Ok(written) => written,
                // Why accept this refusal: a `tfdt` is the sum of the
                // durations of the samples before it, §8.8.12, and the source
                // jumps.
                Err(error)
                    if matches!(
                        error.downcast_ref(),
                        Some(structure_data::Error::DecodingTimeGap { .. })
                    ) =>
                {
                    refused += 1;
                    continue;
                }
                Err(error) => {
                    differences.push(format!("{}: {error:?}", file.name));
                    continue;
                }
            };
            written_files += 1;
            let while_written = read(&fragmented).map(|(_, samples)| samples);
            let once_ended = read(&finished).map(|(_, samples)| samples);
            if while_written.as_ref() != Some(&samples) {
                differences.push(format!(
                    "{}: the samples read while written differ",
                    file.name
                ));
            }
            if once_ended != Some(counted_from_the_first(&samples)) {
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
