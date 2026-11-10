//! A movie that reads as a fragmented movie file while it is written, and as a movie file once it ends, written from the layers of `isobmff` alone
//!
//! The example reads a file with the demuxer, then writes its samples again
//! without a muxer, with only the movie fragment writer and the sample table
//! writer of [`structure_data`], the `mdat` header of
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
//! `cargo run --example hybrid -- <input> <output> [samples_per_fragment]`

use std::error::Error;

use isobmff::boxes::{FileTypeBox, Free, FreeSpaceBox, MovieBox, MovieExtendsBox, TrackExtendsBox};
use isobmff::demuxer::Event;
use isobmff::framing::media_data_header;
use isobmff::structure_data::{MovieFragmentWriter, SampleTableWriter, TrackSampleDescription};
use isobmff::{BoxDefinition, BoxEncode, Demuxer, DemuxerConfig, SampleLocation, SampleProperties};

// Why not take the limit from the muxer configuration: the example writes
// without a muxer; this is the limit a muxer takes by default.
/// The most samples the writers take of a fragment, and of a track
const SAMPLE_COUNT_LIMIT: u64 = 1 << 20;

/// What the structure-data declares of each sample of a file, with its bytes, in file order
type Samples = Vec<(SampleProperties, Vec<u8>)>;

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
    let (file_type, movie, samples) =
        read(&std::fs::read(input)?).ok_or("no ftyp or no moov, or a box that does not read")?;
    let (_, written) = write_hybrid(&file_type, &movie, &samples, samples_per_fragment)?;
    std::fs::write(output, written)?;
    Ok(())
}

/// Reads `file` where the demuxer asks, every track subscribed to: its `ftyp`, its `moov`, and every sample with its bytes in file order, or `None` where it lacks one of the boxes or holds what the demuxer cannot read
fn read(file: &[u8]) -> Option<(FileTypeBox, MovieBox, Samples)> {
    let mut demuxer = Demuxer::new(DemuxerConfig::default());
    let (mut file_type, mut movie, mut samples, mut readable) = (None, None, Vec::new(), true);
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
                Event::Error(_) => readable = false,
                _ => {}
            }
        }
        let Some(required) = demuxer.required_input() else {
            break;
        };
        match file.get(usize::try_from(required.position).ok()?..) {
            Some(bytes) if !bytes.is_empty() => demuxer.handle_input(required.position, bytes),
            _ => demuxer.finish(),
        }
    }
    readable.then_some((file_type?, movie?, samples))
}

/// Writes `samples`, of the tracks of `movie`, after `file_type`, a fragment for every `samples_per_fragment` of them: returns the bytes as a fragmented movie file, and the file once a `moov` declares every sample
fn write_hybrid(
    file_type: &FileTypeBox,
    movie: &MovieBox,
    samples: &[(SampleProperties, Vec<u8>)],
    samples_per_fragment: usize,
) -> Result<(Vec<u8>, Vec<u8>), Box<dyn Error>> {
    let tracks: Vec<TrackSampleDescription<'_>> = movie
        .trak
        .iter()
        .map(|trak| TrackSampleDescription {
            track_id: trak.tkhd.track_id,
            stsd: &trak.mdia.minf.stbl.stsd,
            dref: &trak.mdia.minf.dinf.dref,
        })
        .collect();
    let mut fragments = MovieFragmentWriter::new(&tracks, SAMPLE_COUNT_LIMIT);
    let mut tables: Vec<SampleTableWriter> = tracks
        .iter()
        .map(|track| SampleTableWriter::new(track, SAMPLE_COUNT_LIMIT))
        .collect();
    let mut first_decoding_times = vec![None; tracks.len()];

    // The `ftyp`, the `free` to write the `mdat` header over, and the `moov` of no sample
    let mut written = Vec::new();
    append(file_type, &mut written)?;
    let free_offset = written.len();
    let mut free = FreeSpaceBox::<Free>::default();
    free.data = vec![0; MEDIA_DATA_HEADER_LENGTH - 8];
    append(&free, &mut written)?;
    append(&fragmented_movie_box(movie, &tracks), &mut written)?;

    // Each fragment: its `moof`, the header of its `mdat`, its samples
    for group in samples.chunks(samples_per_fragment.max(1)) {
        let mut media_data = Vec::new();
        let mut placed = Vec::new();
        for (properties, bytes) in group {
            let location = SampleLocation {
                properties: *properties,
                offset: u64::try_from(media_data.len())?,
                size: u32::try_from(bytes.len())?,
            };
            fragments.handle_location(location)?;
            let track = movie
                .trak
                .iter()
                .position(|trak| trak.tkhd.track_id == properties.track_id)
                .ok_or("a sample of a track the moov does not have")?;
            placed.push((track, location));
            media_data.extend_from_slice(bytes);
        }
        append(&fragments.finish(), &mut written)?;
        written.extend_from_slice(
            &media_data_header(u64::try_from(media_data.len())?).ok_or("a fragment too long")?,
        );
        let media_data_offset = u64::try_from(written.len())?;
        written.extend_from_slice(&media_data);

        // Where each sample now lies in the file, for the sample table of its track
        for (track, location) in placed {
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
    }
    let fragmented = written.clone();

    // The `moov` that declares every sample, and the `mdat` header over the `free`
    let mut moov = movie.clone();
    let edits_run_to_the_end = moov.mvex.take().is_some_and(|mvex| mvex.mehd.is_none());
    for (trak, table) in moov.trak.iter_mut().zip(&mut tables) {
        trak.mdia.minf.stbl = table.finish();
    }
    declare_durations(&mut moov, edits_run_to_the_end);
    let payload_length = written.len() - free_offset - MEDIA_DATA_HEADER_LENGTH;
    let header = media_data_header(u64::try_from(payload_length)?).ok_or("a file too long")?;
    written[free_offset..free_offset + MEDIA_DATA_HEADER_LENGTH].copy_from_slice(&header);
    append(&moov, &mut written)?;
    Ok((fragmented, written))
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
// the end of what is written.
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
    use std::path::Path;

    use isobmff::SampleProperties;
    use isobmff::framing::{BoxReader, Event, FramerConfig};
    use isobmff::structure_data;
    use isobmff_test_support::{conformance_files, ffprobe, ffprobe_packets};

    use super::{read, write_hybrid};

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
            let (file_type, movie, samples) = read(file).unwrap();
            let (fragmented, finished) = write_hybrid(&file_type, &movie, &samples, 16).unwrap();

            let (_, movie_while_written, while_written) = read(&fragmented).unwrap();
            assert_eq!(
                (while_written, movie_while_written.mvex.is_some()),
                (samples.clone(), true)
            );

            let (_, movie_once_ended, once_ended) = read(&finished).unwrap();
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
            let Some((file_type, movie, samples)) = read(&file.bytes) else {
                continue;
            };
            if samples.is_empty() {
                continue;
            }
            let (fragmented, finished) = match write_hybrid(&file_type, &movie, &samples, 30) {
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
            let while_written = read(&fragmented).map(|(_, _, samples)| samples);
            let once_ended = read(&finished).map(|(_, _, samples)| samples);
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
