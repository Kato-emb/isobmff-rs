//! A movie file whose `moov` is written last, into space reserved for it before the media data, written from the layers of `isobmff` alone
//!
//! The example reads a file with the demuxer where it asks, and writes each
//! sample it lends without a muxer, with only the sample table writer,
//! [`media_duration`] and [`composition_end`] of [`structure_data`], the box
//! header [`BoxHeader`], and the boxes:
//!
//! 1. an `ftyp`, then a `free` (§8.1.2) of `reserved_bytes`, the space kept
//!    for the `moov`;
//! 2. the header of an `mdat` (§8.1.1) with a `largesize`, then the bytes of
//!    each sample as the demuxer lends them;
//! 3. once every sample is written, the `moov` that declares them all, and
//!    over the first header of the `mdat` its length.
//!
//! Where the `moov` goes depends on how long it turns out, `length`:
//!
//! * `length` + 8 at most `reserved_bytes`: over the start of the `free`,
//!   followed by a `free` of what is left, so the `moov` comes before the
//!   media data;
//! * `length` equal to `reserved_bytes`: over the whole `free`;
//! * otherwise, where what is left would be too short for a `free`, or the
//!   `moov` is longer than the space: after the `mdat`, at the end of the
//!   file, the `free` left as it is. The file is a movie file still, its
//!   `moov` last.
//!
//! The file reads as a movie file only once it ends: the `moov` is written
//! last, so a writing cut off leaves a file without one, unlike the hybrid
//! example, whose fragments read while it is written. Memory is bounded by
//! the limits of [`DemuxerConfig`] and some tens of bytes for each sample of
//! the sample tables, whatever the length of the file.
//!
//! The output is a new file: an output that exists already is refused, so
//! the input is never written over, whatever path names it.
//!
//! `cargo run --example reserved_moov -- <input> <output> [reserved_bytes]`

use std::error::Error;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use isobmff::boxes::MovieBox;
use isobmff::demuxer::Event;
use isobmff::structure_data::{
    SampleTableWriter, TrackSampleDescription, composition_end, media_duration,
};
use isobmff::{
    BoxEncode, BoxHeader, BoxSize, BoxType, CompactSize, Demuxer, DemuxerConfig, ExtendedSize,
    SampleLocation, SampleProperties,
};

// Why not take the limit from the muxer configuration: the example writes
// without a muxer; this is the limit a muxer takes by default.
/// The most samples the sample table of a track takes
const SAMPLE_COUNT_LIMIT: u64 = 1 << 20;

/// The space reserved for the `moov` when the command line gives none: 1 MiB
///
/// The sample tables take some bytes a sample, so 1 MiB holds the `moov` of
/// a movie of some tens of minutes, a video and an audio track at common
/// rates. A longer movie takes more, or has its `moov` at the end.
const RESERVED_BYTES: u32 = 1 << 20;

/// Length of the header of a `free` with a 32-bit size, the shortest a `free` can be
const FREE_HEADER_LENGTH: u64 = 8;

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args().skip(1);
    let usage = "usage: reserved_moov <input> <output> [reserved_bytes]";
    let (input, output) = (
        arguments.next().ok_or(usage)?,
        arguments.next().ok_or(usage)?,
    );
    let reserved_bytes = arguments
        .next()
        .map_or(Ok(RESERVED_BYTES), |bytes| bytes.parse::<u32>())?;
    run(Path::new(&input), Path::new(&output), reserved_bytes)
}

/// Writes the samples of the file at `input` to a new file at `output`, `reserved_bytes` reserved for the `moov` before them
fn run(input: &Path, output: &Path, reserved_bytes: u32) -> Result<(), Box<dyn Error>> {
    let (input, output) = (
        std::fs::File::open(input)?,
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)?,
    );
    // Why buffer the output: the bytes of each sample are written as the
    // demuxer lends them, a write for each sample.
    reserved_moov(input, std::io::BufWriter::new(output), reserved_bytes)
}

/// Writes the samples of `input` to `output` after a `free` of `reserved_bytes` and an `mdat` header, then the `moov` that declares them into the `free`, or after them where it does not fit
fn reserved_moov(
    mut input: impl Read + Seek,
    mut output: impl Write + Seek,
    reserved_bytes: u32,
) -> Result<(), Box<dyn Error>> {
    let free_size =
        CompactSize::new(reserved_bytes).ok_or("fewer reserved bytes than a free takes")?;
    let mut demuxer = Demuxer::new(DemuxerConfig::default());
    let mut buffer = vec![0; 64 * 1024];
    let mut file_type = None;
    // Set once the `moov` is read: the movie, and where the `free` and the
    // `mdat` lie; the writer of the sample table of each track, and the
    // decoding time of its first sample; and where the next sample goes
    let mut movie: Option<(MovieBox, u64, u64)> = None;
    let mut tables: Vec<(SampleTableWriter, Option<u64>)> = Vec::new();
    let mut position = 0_u64;
    loop {
        while let Some(event) = demuxer.poll_event() {
            match event {
                Event::FileType(ftyp) => file_type = Some(ftyp),
                // The `ftyp`, the `free` the `moov` is to be written over, and the header of the `mdat`
                Event::Movie(moov) => {
                    let ftyp = file_type.take().ok_or("a moov before any ftyp")?;
                    let mut bytes = vec![0; usize::try_from(ftyp.encoded_len())?];
                    ftyp.encode(&mut bytes)?;
                    output.write_all(&bytes)?;
                    let free_offset = output.stream_position()?;
                    output.write_all(&header(*b"free", BoxSize::Compact(free_size))?)?;
                    std::io::copy(
                        &mut std::io::repeat(0)
                            .take(u64::from(reserved_bytes).saturating_sub(FREE_HEADER_LENGTH)),
                        &mut output,
                    )?;
                    let media_data_offset = output.stream_position()?;
                    let empty = ExtendedSize::new(16).ok_or("an mdat shorter than its header")?;
                    output.write_all(&header(*b"mdat", BoxSize::Extended(empty))?)?;
                    position = output.stream_position()?;
                    tables = moov
                        .trak
                        .iter()
                        .map(|trak| {
                            let track = TrackSampleDescription {
                                track_id: trak.tkhd.track_id,
                                stsd: &trak.mdia.minf.stbl.stsd,
                                dref: &trak.mdia.minf.dinf.dref,
                            };
                            (SampleTableWriter::new(&track, SAMPLE_COUNT_LIMIT), None)
                        })
                        .collect();
                    for trak in &moov.trak {
                        demuxer.subscribe_track(trak.tkhd.track_id);
                    }
                    movie = Some((*moov, free_offset, media_data_offset));
                }
                // Each sample is written where the media data has come to, and its table takes it there
                Event::Sample(sample) => {
                    let (moov, _, _) = movie.as_ref().ok_or("a sample before any moov")?;
                    let properties = sample.properties;
                    let (table, first) = moov
                        .trak
                        .iter()
                        .position(|trak| trak.tkhd.track_id == properties.track_id)
                        .and_then(|track| tables.get_mut(track))
                        .ok_or("a sample of a track the moov does not have")?;
                    let first = *first.get_or_insert(properties.decoding_time);
                    table.handle_location(SampleLocation {
                        properties: SampleProperties {
                            decoding_time: properties.decoding_time.saturating_sub(first),
                            ..properties
                        },
                        offset: position,
                        size: u32::try_from(sample.bytes.len())?,
                    })?;
                    output.write_all(sample.bytes)?;
                    position = position.saturating_add(u64::try_from(sample.bytes.len())?);
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
    let (mut moov, free_offset, media_data_offset) = movie.ok_or("no moov")?;

    // The `moov` that declares every sample
    moov.mvex = None;
    for (trak, (table, _)) in moov.trak.iter_mut().zip(&mut tables) {
        trak.mdia.minf.stbl = table.finish();
    }
    declare_durations(&mut moov);
    let length = moov.encoded_len();
    let mut bytes = vec![0; usize::try_from(length)?];
    moov.encode(&mut bytes)?;
    let at = match u64::from(reserved_bytes).checked_sub(length) {
        Some(0) => free_offset,
        Some(rest) if rest >= FREE_HEADER_LENGTH => {
            let rest = CompactSize::new(u32::try_from(rest)?).ok_or("a free too short")?;
            bytes.extend(header(*b"free", BoxSize::Compact(rest))?);
            free_offset
        }
        _ => position,
    };
    output.seek(SeekFrom::Start(at))?;
    output.write_all(&bytes)?;

    // The header of the `mdat` was written first, as long as one with a
    // `largesize`; now that the length is known, it is written over
    let size =
        ExtendedSize::new(position.saturating_sub(media_data_offset)).ok_or("a file too long")?;
    output.seek(SeekFrom::Start(media_data_offset))?;
    output.write_all(&header(*b"mdat", BoxSize::Extended(size))?)?;
    output.flush()?;
    Ok(())
}

/// Returns the header of a box of type `box_type` and `size`
fn header(box_type: [u8; 4], size: BoxSize) -> Result<Vec<u8>, Box<dyn Error>> {
    let header =
        BoxHeader::new(BoxType::compact(box_type), size).ok_or("a box shorter than its header")?;
    let mut bytes = vec![0; usize::try_from(header.encoded_len())?];
    header.encode(&mut bytes)?;
    Ok(bytes)
}

/// Sets the durations of `moov`, of each of its tracks and of their media to those the sample tables and the edit lists give, and each header to the version they need
///
/// The media lasts as long as its samples, §8.4.2; a track as long as its
/// edits, or without an edit list as long as its media, §8.3.2; the movie as
/// long as its longest track, §8.2.2. A header takes version 1 where its
/// duration or one of its times does not fit 32 bits.
fn declare_durations(moov: &mut MovieBox) {
    let version =
        |values: [u64; 3]| u8::from(values.iter().any(|&value| value > u64::from(u32::MAX)));
    let movie_timescale = u128::from(moov.mvhd.timescale);
    for trak in &mut moov.trak {
        let stbl = &trak.mdia.minf.stbl;
        let (media, composed) = (media_duration(stbl), composition_end(stbl));
        let media_timescale = u128::from(trak.mdia.mdhd.timescale).max(1);
        let in_movie_timescale = |duration: u64| {
            u64::try_from(
                u128::from(duration)
                    .saturating_mul(movie_timescale)
                    .div_ceil(media_timescale),
            )
            .unwrap_or(u64::MAX)
        };
        trak.tkhd.duration = match trak.edts.as_mut().and_then(|edts| edts.elst.as_mut()) {
            Some(elst) => {
                // A last edit of duration 0 runs to the end of the media of a
                // fragmented movie, §8.6.6.1; a movie file closes it where the
                // composition of the samples ends.
                if let Some(last) = elst
                    .entries
                    .last_mut()
                    .filter(|last| last.edit_duration == 0 && last.media_time >= 0)
                {
                    last.edit_duration =
                        in_movie_timescale(composed.saturating_sub(last.media_time.unsigned_abs()));
                    elst.version = elst.version.max(version([last.edit_duration, 0, 0]));
                }
                elst.entries.iter().fold(0_u64, |total, entry| {
                    total.saturating_add(entry.edit_duration)
                })
            }
            None => in_movie_timescale(media),
        };
        let tkhd = &mut trak.tkhd;
        tkhd.version = version([tkhd.duration, tkhd.creation_time, tkhd.modification_time]);
        let mdhd = &mut trak.mdia.mdhd;
        mdhd.duration = media;
        mdhd.version = version([mdhd.duration, mdhd.creation_time, mdhd.modification_time]);
    }
    let mvhd = &mut moov.mvhd;
    mvhd.duration = moov
        .trak
        .iter()
        .map(|trak| trak.tkhd.duration)
        .max()
        .unwrap_or(0);
    mvhd.version = version([mvhd.duration, mvhd.creation_time, mvhd.modification_time]);
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::error::Error;
    use std::io::Cursor;
    use std::path::Path;

    use isobmff::framing::{BoxReader, Event, FramerConfig};
    use isobmff::{Demuxer, DemuxerConfig, SampleProperties, demuxer, structure_data};
    use isobmff_test_support::{conformance_files, ffprobe, ffprobe_packets};

    use super::{RESERVED_BYTES, reserved_moov};

    /// What the structure-data declares of each sample of a file, with its bytes, in file order
    type Samples = Vec<(SampleProperties, Vec<u8>)>;

    /// Every sample of `file` with its bytes, in file order, as the demuxer lends them, or `None` where it reports what it cannot read or finds no `moov`
    fn read(file: &[u8]) -> Option<Samples> {
        let mut demuxer = Demuxer::new(DemuxerConfig::default());
        let (mut movie, mut samples, mut readable) = (false, Vec::new(), true);
        loop {
            while let Some(event) = demuxer.poll_event() {
                match event {
                    demuxer::Event::Movie(moov) => {
                        for trak in &moov.trak {
                            demuxer.subscribe_track(trak.tkhd.track_id);
                        }
                        movie = true;
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
        (readable && movie).then_some(samples)
    }

    /// What the example writes of `file`, `reserved_bytes` reserved for the `moov`
    fn written(file: &[u8], reserved_bytes: u32) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut output = Cursor::new(Vec::new());
        reserved_moov(Cursor::new(file), &mut output, reserved_bytes)?;
        Ok(output.into_inner())
    }

    /// The fixtures of the reading tests, two movie files and two of movie fragments
    const FIXTURES: [&[u8]; 4] = [
        include_bytes!("../crates/isobmff/tests/fixtures/movie_moov_first.mp4"),
        include_bytes!("../crates/isobmff/tests/fixtures/movie_moov_last.mp4"),
        include_bytes!("../crates/isobmff/tests/fixtures/fragmented.mp4"),
        include_bytes!("../crates/isobmff/tests/fixtures/segment.mp4"),
    ];

    /// The samples of `source`, the decoding times of each track counted from that of its first sample, as a sample table declares them
    fn counted_from_the_first(samples: &[(SampleProperties, Vec<u8>)]) -> Samples {
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

    /// The type of each box at file level of `file`, and its file offset
    fn top_level_boxes(file: &[u8]) -> Vec<(String, u64)> {
        let mut config = FramerConfig::new(0);
        config.gathered_box_types.clear();
        let mut reader = BoxReader::new(config);
        let mut boxes = Vec::new();
        loop {
            while let Some(event) = reader.poll_event() {
                if let Event::Box { offset, header } = event {
                    boxes.push((header.box_type().to_string(), offset));
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
        boxes
    }

    /// The type of each box at file level of `file`
    fn top_level_box_types(file: &[u8]) -> Vec<String> {
        top_level_boxes(file)
            .into_iter()
            .map(|(box_type, _)| box_type)
            .collect()
    }

    #[test]
    fn a_moov_that_fits_the_space_reserved_comes_before_the_media_data_and_a_free_of_what_is_left()
    {
        for file in FIXTURES {
            let finished = written(file, RESERVED_BYTES).unwrap();
            assert_eq!(
                (top_level_box_types(&finished), read(&finished)),
                (
                    ["ftyp", "moov", "free", "mdat"].map(String::from).to_vec(),
                    Some(counted_from_the_first(&read(file).unwrap()))
                )
            );
        }
    }

    /// With the `moov` of the fixture `length` bytes long: room for a `free` after it, room for it alone, room left too short for a `free`, too little room, and the least a `free` takes
    #[test]
    fn a_moov_takes_the_space_reserved_where_it_fits_and_goes_last_where_it_does_not() {
        let file = FIXTURES[1];
        let boxes = top_level_boxes(&written(file, RESERVED_BYTES).unwrap());
        let length = u32::try_from(boxes[2].1 - boxes[1].1).unwrap();
        let samples = Some(counted_from_the_first(&read(file).unwrap()));
        let layout_of = |reserved_bytes: u32| {
            let finished = written(file, reserved_bytes).unwrap();
            (top_level_box_types(&finished), read(&finished))
        };
        let layout = |types: &[&str]| {
            (
                types.iter().map(|box_type| box_type.to_string()).collect(),
                samples.clone(),
            )
        };
        assert_eq!(
            [
                layout_of(length + 8),
                layout_of(length),
                layout_of(length + 4),
                layout_of(length - 1),
                layout_of(8),
            ],
            [
                layout(&["ftyp", "moov", "free", "mdat"]),
                layout(&["ftyp", "moov", "mdat"]),
                layout(&["ftyp", "free", "mdat", "moov"]),
                layout(&["ftyp", "free", "mdat", "moov"]),
                layout(&["ftyp", "free", "mdat", "moov"]),
            ]
        );
    }

    #[test]
    fn fewer_reserved_bytes_than_a_free_takes_are_refused() {
        assert!(written(FIXTURES[1], 7).is_err());
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
    fn the_conformance_files_written_with_their_moov_in_the_space_reserved_read_as_the_same_samples_and_packets()
     {
        let directory =
            std::env::temp_dir().join(format!("isobmff-reserved-moov-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let (mut differences, mut written_files, mut compared, mut refused) = (Vec::new(), 0, 0, 0);
        for file in conformance_files() {
            // Why skip these: a file the demuxer reports an error in, such as
            // one whose samples lie in another file, has none to write.
            let Some(samples) = read(&file.bytes) else {
                continue;
            };
            if samples.is_empty() {
                continue;
            }
            let finished = match written(&file.bytes, RESERVED_BYTES) {
                Ok(finished) => finished,
                // Why accept this refusal: a sample table declares decoding
                // times as the sum of the durations before them, §8.6.1.2,
                // and the movie fragments of the source jump.
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
            let types = top_level_box_types(&finished);
            if types.get(1).map(String::as_str) != Some("moov") {
                differences.push(format!("{}: the moov is not first: {types:?}", file.name));
            }
            if read(&finished) != Some(counted_from_the_first(&samples)) {
                differences.push(format!("{}: the samples read back differ", file.name));
            }
            // Why compare these with this crate's demuxer alone: ffprobe
            // refuses to read the source.
            if !ffprobe(&["-v", "error"], &file.path).status.success() {
                continue;
            }
            let path = directory.join(&file.name);
            std::fs::write(&path, &finished).unwrap();
            compared += 1;
            let probed = |path: &Path| (ffprobe_packets(&[], path).len(), packets(path));
            let (ours, theirs) = (probed(&path), probed(&file.path));
            if ours != theirs {
                differences.push(format!(
                    "{}: ffprobe presents {} packets, {} of the source, or reads other packets with the edits ignored",
                    file.name, ours.0, theirs.0
                ));
            }
        }
        std::fs::remove_dir_all(&directory).unwrap();
        println!(
            "{written_files} files written, {compared} compared with ffprobe, {refused} refused for a gap in decoding times"
        );
        assert_eq!(differences, Vec::<String>::new());
        assert!(written_files > 0);
    }

    #[test]
    fn an_output_that_already_exists_is_refused_and_the_input_is_left_as_it_was() {
        let directory = std::env::temp_dir().join(format!(
            "isobmff-reserved-moov-output-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let (input, link, other) = (
            directory.join("input.mp4"),
            directory.join("link.mp4"),
            directory.join("other.mp4"),
        );
        std::fs::write(&input, FIXTURES[1]).unwrap();
        std::fs::hard_link(&input, &link).unwrap();
        std::fs::write(&other, b"other").unwrap();
        for output in [&input, &link, &other] {
            let refused = super::run(&input, output, RESERVED_BYTES).is_err();
            assert_eq!(
                (
                    refused,
                    std::fs::read(&input).unwrap(),
                    std::fs::read(&other).unwrap()
                ),
                (true, FIXTURES[1].to_vec(), b"other".to_vec())
            );
        }
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
