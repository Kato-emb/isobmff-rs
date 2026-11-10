//! A movie that reads as a fragmented movie file while it is written, and as a movie file once it ends, written from the layers of `isobmff` alone
//!
//! The example reads a file with the demuxer where it asks, and writes each
//! sample it lends without a muxer, with only the movie fragment writer and
//! the sample table writer of [`structure_data`], the box header
//! [`BoxHeader`], and the boxes:
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
//! While it is written, the file is a whole fragmented movie file each time a
//! fragment has been written: a writing cut off, by a crash or a lost
//! connection, leaves a file that reads up to the last fragment written, and
//! loses only the fragment being written. `finish` then appends the `moov`
//! and turns the `free` into the header of an `mdat`, which makes the file a
//! movie file.
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
use isobmff::structure_data::{MovieFragmentWriter, SampleTableWriter, TrackSampleDescription};
use isobmff::{
    BoxDefinition, BoxEncode, BoxHeader, BoxSize, BoxType, Demuxer, DemuxerConfig, ExtendedSize,
    SampleLocation, SampleProperties,
};

// Why not take the limit from the muxer configuration: the example writes
// without a muxer; this is the limit a muxer takes by default.
/// The most samples the writers take of a fragment, and of a track
const SAMPLE_COUNT_LIMIT: u64 = 1 << 20;

/// Length of the header of an `mdat` with a `largesize`, which the `free` that opens the movie is as long as
const MEDIA_DATA_HEADER_LENGTH: usize = 16;

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args().skip(1);
    let usage = "usage: hybrid <input> <output> [samples_per_fragment]";
    let (input, output) = (
        arguments.next().ok_or(usage)?,
        arguments.next().ok_or(usage)?,
    );
    if std::fs::canonicalize(&output).ok() == Some(std::fs::canonicalize(&input)?) {
        return Err("the output is the input, which writing it would empty".into());
    }
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
    // The `free` was written first, as long as an `mdat` header with a
    // `largesize`; now that the length is known, the header is written over it
    let size =
        ExtendedSize::new(output.stream_position()? - free_offset).ok_or("a file too long")?;
    let header = BoxHeader::new(BoxType::compact(*b"mdat"), BoxSize::Extended(size))
        .ok_or("a file too long")?;
    let mut bytes = [0; MEDIA_DATA_HEADER_LENGTH];
    header.encode(&mut bytes)?;
    append(&moov, &mut out)?;
    output.write_all(&out)?;
    output.seek(SeekFrom::Start(free_offset))?;
    output.write_all(&bytes)?;
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
    // The `moof` declares its samples after an `mdat` header with a `largesize`
    let size = ExtendedSize::new(u64::try_from(MEDIA_DATA_HEADER_LENGTH + media_data.len())?)
        .ok_or("a fragment too long")?;
    let header = BoxHeader::new(BoxType::compact(*b"mdat"), BoxSize::Extended(size))
        .ok_or("a fragment too long")?;
    let start = out.len();
    out.resize(start + MEDIA_DATA_HEADER_LENGTH, 0);
    header.encode(&mut out[start..])?;
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
    // The file is now a whole fragmented movie file, up to this fragment
    output.flush()?;
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

/// Sets the durations of the media of each track of `moov`, of the track and of the movie, to those its sample tables and edit lists give, each box in the version its duration and times need
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
        let mdhd = &mut trak.mdia.mdhd;
        mdhd.version = u8::from(
            [media_duration, mdhd.creation_time, mdhd.modification_time]
                .iter()
                .any(|&value| value > u64::from(u32::MAX)),
        );
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
        let tkhd = &mut trak.tkhd;
        tkhd.version = u8::from(
            [duration, tkhd.creation_time, tkhd.modification_time]
                .iter()
                .any(|&value| value > u64::from(u32::MAX)),
        );
    }
    moov.mvhd.duration = moov
        .trak
        .iter()
        .map(|trak| trak.tkhd.duration)
        .max()
        .unwrap_or(0);
    let mvhd = &mut moov.mvhd;
    mvhd.version = u8::from(
        [mvhd.duration, mvhd.creation_time, mvhd.modification_time]
            .iter()
            .any(|&value| value > u64::from(u32::MAX)),
    );
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

    /// The segment fixture, its `mvhd`, and each `tkhd` and `mdhd`, created and modified at a time past 32 bits, so of version 1
    fn with_times_past_32_bits() -> Vec<u8> {
        use isobmff::BoxDecode;
        use isobmff::boxes::FileTypeBox;

        let file = FIXTURES[3];
        let (_, after_ftyp) = FileTypeBox::decode(file).unwrap();
        let (mut moov, after_moov) = MovieBox::decode(after_ftyp).unwrap();
        let time = 1 << 32;
        moov.mvhd.version = 1;
        (moov.mvhd.creation_time, moov.mvhd.modification_time) = (time, time);
        for trak in &mut moov.trak {
            trak.tkhd.version = 1;
            (trak.tkhd.creation_time, trak.tkhd.modification_time) = (time, time);
            trak.mdia.mdhd.version = 1;
            (
                trak.mdia.mdhd.creation_time,
                trak.mdia.mdhd.modification_time,
            ) = (time, time);
        }
        let mut written = file[..file.len() - after_ftyp.len()].to_vec();
        super::append(&moov, &mut written).unwrap();
        written.extend_from_slice(after_moov);
        written
    }

    #[test]
    fn a_movie_created_at_a_time_past_32_bits_is_written_with_its_times() {
        let file = with_times_past_32_bits();
        let (movie, samples) = read(&file).unwrap();
        let (_, finished) = write_hybrid(&file, 16).unwrap();
        let (written, read_back) = read(&finished).unwrap();
        let times = |moov: &MovieBox| {
            let mut times = vec![(moov.mvhd.creation_time, moov.mvhd.modification_time)];
            for trak in &moov.trak {
                times.push((trak.tkhd.creation_time, trak.tkhd.modification_time));
                times.push((
                    trak.mdia.mdhd.creation_time,
                    trak.mdia.mdhd.modification_time,
                ));
            }
            times
        };
        assert_eq!(
            (read_back.len(), times(&written)),
            (samples.len(), times(&movie))
        );
    }

    /// What the example writes of `file`, a fragment for every `samples_per_fragment` samples: the file as it stood once its last fragment was written, before its last `moov` and its `mdat` header were, and the file once they were
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
    fn a_movie_cut_off_after_any_fragment_reads_up_to_it_and_once_it_ends_reads_as_a_movie_file() {
        for file in FIXTURES {
            let (_, samples) = read(file).unwrap();
            let (fragmented, finished) = write_hybrid(file, 16).unwrap();

            // Where each fragment ends: its `moof`, then its `mdat`, then the next box or the end
            let boxes = top_level_boxes(&fragmented);
            let ends_of_fragments: Vec<u64> = (0..boxes.len())
                .filter(|&index| {
                    boxes[index].0 == "moof"
                        && boxes.get(index + 1).is_some_and(|next| next.0 == "mdat")
                })
                .map(|index| {
                    boxes
                        .get(index + 2)
                        .map_or(u64::try_from(fragmented.len()).unwrap(), |(_, offset)| {
                            *offset
                        })
                })
                .collect();
            assert_eq!(ends_of_fragments.len(), samples.len().div_ceil(16));
            for (fragments, end) in (1..).zip(&ends_of_fragments) {
                let cut_off = &fragmented[..usize::try_from(*end).unwrap()];
                let (movie_while_written, while_written) = read(cut_off).unwrap();
                let written = samples.len().min(fragments * 16);
                assert_eq!(
                    (while_written.as_slice(), movie_while_written.mvex.is_some()),
                    (&samples[..written], true)
                );
            }

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
