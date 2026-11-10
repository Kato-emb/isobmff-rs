//! A movie file laid out again with its `moov` before its media data, from the layers of `isobmff` alone
//!
//! The example reads the boxes at file level of a movie file with the box
//! reader of [`framing`](isobmff::framing), where it asks, gathering only the
//! `ftyp`, the `moov` and a `meta`, and skipping the rest by their headers. It
//! then writes:
//!
//! 1. the `ftyp`, where it is the first box;
//! 2. the `moov`, its chunk offsets (`stco` / `co64`, §8.7.5) and sample
//!    auxiliary information offsets (`saio`, §8.7.9) moved to where the media
//!    data they point at now lies;
//! 3. every other box, in the order it came in, copied from the input.
//!
//! The media data is copied through a buffer of fixed length, never held, so
//! memory is bounded by the `moov` and the buffer, whatever the length of the
//! file. A file whose `moov` already comes before every `mdat` is copied as it
//! is. A fragmented movie file, one whose `moov` holds an offset into the
//! `moov` itself, and one with an item that lies in the file (§8.11.3) are
//! refused: moving the `moov` moves what they point at. So is one with a
//! track whose data references name media data elsewhere than in the file
//! (another file, or an `IdentifiedMediaDataBox`, §8.7.2): which of its
//! offsets to move would need each chunk resolved to its data reference.
//!
//! The output is a new file: an output that exists already is refused, so
//! the input is never written over, whatever path names it.
//!
//! `cargo run --example fast_start -- <input> <output>`

use std::error::Error;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use isobmff::boxes::{ChunkLargeOffsetBox, ChunkOffsetBox, ChunkOffsetVariant, MetaBox, MovieBox};
use isobmff::framing::{BoxReader, Event, FramerConfig};
use isobmff::{BoxDecode, BoxEncode, BoxType};

// Why not take the limit from the demuxer configuration: the example reads
// without a demuxer; this is the limit a demuxer takes by default.
/// The most bytes the payload of a `moov` or a `meta` may hold
const PAYLOAD_LIMIT: u64 = 16 * 1024 * 1024;

const FILE_TYPE: BoxType = BoxType::compact(*b"ftyp");
const MOVIE: BoxType = BoxType::compact(*b"moov");
const META: BoxType = BoxType::compact(*b"meta");
const MEDIA_DATA: BoxType = BoxType::compact(*b"mdat");

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args().skip(1);
    let usage = "usage: fast_start <input> <output>";
    let (input, output) = (
        arguments.next().ok_or(usage)?,
        arguments.next().ok_or(usage)?,
    );
    run(Path::new(&input), Path::new(&output))
}

/// Writes the file at `input` to a new file at `output`, its `moov` first
fn run(input: &Path, output: &Path) -> Result<(), Box<dyn Error>> {
    let (input, output) = (
        std::fs::File::open(input)?,
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)?,
    );
    fast_start(input, output)
}

/// Writes `input` to `output` with its `moov` after its `ftyp` and before every other box, the offsets of the `moov` moved where the boxes they point into now lie
fn fast_start(
    mut input: impl Read + Seek,
    mut output: impl Write + Seek,
) -> Result<(), Box<dyn Error>> {
    let mut config = FramerConfig::new(PAYLOAD_LIMIT);
    config.gathered_box_types = vec![FILE_TYPE, MOVIE, META];
    let mut reader = BoxReader::new(config);
    let mut buffer = vec![0; 64 * 1024];
    // Where the `ftyp` ends, if it is the first box; the `moov` and where it
    // lies; and whether an `mdat` comes before it
    let (mut head_end, mut movie, mut media_data_first) = (0, None, false);
    loop {
        while let Some(event) = reader.poll_event() {
            match event {
                Event::Gathered {
                    offset,
                    box_type,
                    payload_offset,
                    payload,
                } => {
                    let end = payload_offset + u64::try_from(payload.len())?;
                    match box_type {
                        FILE_TYPE if offset == 0 => head_end = end,
                        MOVIE if movie.is_some() => return Err("a second moov".into()),
                        MOVIE => movie = Some((MovieBox::decode_payload(payload)?, offset, end)),
                        META if lies_in_the_file(&MetaBox::decode_payload(payload)?) => {
                            return Err("an item that lies in the file".into());
                        }
                        _ => {}
                    }
                }
                Event::Box { header, .. } if header.box_type() == MEDIA_DATA => {
                    media_data_first |= movie.is_none();
                }
                Event::Error(error) => return Err(error.into()),
                _ => {}
            }
        }
        let Some(required) = reader.required_input() else {
            break;
        };
        input.seek(SeekFrom::Start(required.position))?;
        match input.read(&mut buffer)? {
            0 => reader.finish(),
            read => reader.handle_input(required.position, &buffer[..read]),
        }
    }
    let (source, moov_offset, moov_end) = movie.ok_or("no moov")?;
    if source.mvex.is_some() {
        return Err("a fragmented movie, whose movie fragments the moov does not declare".into());
    }
    let mut metas = source
        .meta
        .iter()
        .chain(source.trak.iter().filter_map(|trak| trak.meta.as_ref()));
    if metas.any(lies_in_the_file) {
        return Err("an item that lies in the file".into());
    }
    if source.trak.iter().any(|trak| {
        let entries = &trak.mdia.minf.dinf.dref.data_entries;
        entries.iter().any(|entry| !entry.is_in_same_file())
    }) {
        return Err("a track whose media data may lie elsewhere than in the file".into());
    }
    if !media_data_first {
        input.seek(SeekFrom::Start(0))?;
        std::io::copy(&mut input, &mut output)?;
        return Ok(());
    }

    // Why not move every offset by one delta: the boxes between the `ftyp`
    // and the `moov` move by the whole length of the `moov` as it is written,
    // those after it only by how much longer it becomes, and an offset into
    // the `moov` points at nothing once the `moov` moves.
    let mut moov_length = moov_end - moov_offset;
    let moov = loop {
        let mut moov = source.clone();
        let written_length = moov_length;
        map_offsets(&mut moov, |offset| match offset {
            _ if offset < head_end => Some(offset),
            _ if offset < moov_offset => offset.checked_add(written_length),
            _ if offset < moov_end => None,
            _ => offset
                .checked_add(written_length)?
                .checked_sub(moov_end - moov_offset),
        })?;
        moov_length = moov.encoded_len();
        // Why not settle on the first length: a table whose offsets cross 32
        // bits changes its width, which changes the length of the `moov` and
        // moves the offsets again. After the first pass the length moves one
        // way only, so each table changes its width at most once more, and
        // the length settles once none does.
        if moov_length == written_length {
            break moov;
        }
    };

    copy(&mut input, 0, head_end, &mut output)?;
    let mut bytes = vec![0; usize::try_from(moov_length)?];
    moov.encode(&mut bytes)?;
    output.write_all(&bytes)?;
    copy(&mut input, head_end, moov_offset, &mut output)?;
    input.seek(SeekFrom::Start(moov_end))?;
    std::io::copy(&mut input, &mut output)?;
    Ok(())
}

/// Maps every chunk offset and sample auxiliary information offset of `moov` with `map`, each table in its short form where every offset of it fits 32 bits, and in its long form otherwise
fn map_offsets(
    moov: &mut MovieBox,
    map: impl Fn(u64) -> Option<u64>,
) -> Result<(), Box<dyn Error>> {
    let map_all = |offsets: Vec<u64>| -> Result<Vec<u64>, Box<dyn Error>> {
        offsets
            .into_iter()
            .map(|offset| {
                map(offset)
                    .ok_or_else(|| format!("offset {offset} points at nothing once moved").into())
            })
            .collect()
    };
    for trak in &mut moov.trak {
        let stbl = &mut trak.mdia.minf.stbl;
        let offsets = match &stbl.chunk_offset {
            Some(ChunkOffsetVariant::ChunkOffset(stco)) => {
                stco.chunk_offsets.iter().copied().map(u64::from).collect()
            }
            Some(ChunkOffsetVariant::ChunkLargeOffset(co64)) => co64.chunk_offsets.clone(),
            _ => return Err("a track with no chunk offset table".into()),
        };
        let offsets = map_all(offsets)?;
        stbl.chunk_offset = Some(
            match offsets
                .iter()
                .map(|&offset| u32::try_from(offset))
                .collect()
            {
                Ok(compact) => {
                    let mut stco = ChunkOffsetBox::default();
                    stco.chunk_offsets = compact;
                    ChunkOffsetVariant::ChunkOffset(stco)
                }
                Err(_) => {
                    let mut co64 = ChunkLargeOffsetBox::default();
                    co64.chunk_offsets = offsets;
                    ChunkOffsetVariant::ChunkLargeOffset(co64)
                }
            },
        );
        for saio in &mut stbl.saio {
            saio.offsets = map_all(std::mem::take(&mut saio.offsets))?;
            saio.version = u8::from(
                saio.offsets
                    .iter()
                    .any(|&offset| u32::try_from(offset).is_err()),
            );
        }
    }
    Ok(())
}

/// Copies the bytes of `input` from `start` up to `end` to `output`, through a buffer of fixed length
fn copy(
    input: &mut (impl Read + Seek),
    start: u64,
    end: u64,
    output: &mut impl Write,
) -> Result<(), Box<dyn Error>> {
    input.seek(SeekFrom::Start(start))?;
    let copied = std::io::copy(&mut input.take(end - start), output)?;
    if copied != end - start {
        return Err("a file that ends inside a box".into());
    }
    Ok(())
}

/// Whether an item of `meta` lies in the file that holds it, at an offset moving a box would leave behind, §8.11.3
fn lies_in_the_file(meta: &MetaBox) -> bool {
    meta.iloc.iter().flat_map(|iloc| &iloc.items).any(|item| {
        item.construction_method == 0
            && (item.data_reference_index == 0
                || meta
                    .dinf
                    .as_ref()
                    .and_then(|dinf| {
                        dinf.dref
                            .data_entries
                            .get(usize::from(item.data_reference_index) - 1)
                    })
                    .is_some_and(|entry| entry.is_in_same_file()))
    })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use isobmff::boxes::{
        ChunkLargeOffsetBox, ChunkOffsetVariant, DataEntry, DataEntryUrlBox, DataInformationBox,
        ItemLocation, ItemLocationBox, MetaBox, MovieBox,
    };
    use isobmff::framing::{BoxReader, Event, FramerConfig};
    use isobmff::{
        BoxDecode, BoxEncode, Demuxer, DemuxerConfig, SampleProperties, Utf8String, demuxer,
    };
    use isobmff_test_support::{FfprobePacket, conformance_files, ffprobe, ffprobe_packets};

    use super::{fast_start, run};

    const MOOV_FIRST: &[u8] =
        include_bytes!("../crates/isobmff/tests/fixtures/movie_moov_first.mp4");
    const MOOV_LAST: &[u8] = include_bytes!("../crates/isobmff/tests/fixtures/movie_moov_last.mp4");

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

    /// What the example writes of `file`
    fn written(file: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut output = Cursor::new(Vec::new());
        fast_start(Cursor::new(file), &mut output)?;
        Ok(output.into_inner())
    }

    /// The fixture whose `moov` is last, its `moov` changed by `change`
    fn moov_last_with(change: impl FnOnce(&mut MovieBox, u64)) -> Vec<u8> {
        let moov_offset = MOOV_LAST.len() - 1800;
        let (mut moov, _) = MovieBox::decode(&MOOV_LAST[moov_offset..]).unwrap();
        change(&mut moov, u64::try_from(moov_offset).unwrap());
        let mut file = MOOV_LAST[..moov_offset].to_vec();
        let mut bytes = vec![0; usize::try_from(moov.encoded_len()).unwrap()];
        moov.encode(&mut bytes).unwrap();
        file.extend(bytes);
        file
    }

    #[test]
    fn a_movie_with_its_moov_last_is_written_with_it_first_and_the_same_samples() {
        let finished = written(MOOV_LAST).unwrap();
        assert_eq!(
            (top_level_box_types(&finished), read(&finished).unwrap().1),
            (
                ["ftyp", "moov", "free", "mdat"].map(String::from).to_vec(),
                read(MOOV_LAST).unwrap().1
            )
        );
    }

    #[test]
    fn a_movie_with_its_moov_first_is_copied_as_it_is() {
        assert_eq!(written(MOOV_FIRST).unwrap(), MOOV_FIRST);
    }

    #[test]
    fn a_moov_of_64_bit_chunk_offsets_that_fit_32_bits_is_written_shorter_with_the_same_samples() {
        let file = moov_last_with(|moov, _| {
            for trak in &mut moov.trak {
                let stbl = &mut trak.mdia.minf.stbl;
                if let Some(ChunkOffsetVariant::ChunkOffset(stco)) = &stbl.chunk_offset {
                    let mut co64 = ChunkLargeOffsetBox::default();
                    co64.chunk_offsets =
                        stco.chunk_offsets.iter().copied().map(u64::from).collect();
                    stbl.chunk_offset = Some(ChunkOffsetVariant::ChunkLargeOffset(co64));
                }
            }
        });
        let finished = written(&file).unwrap();
        let (moov, samples) = read(&finished).unwrap();
        assert!(finished.len() < file.len());
        assert!(moov.trak.iter().all(|trak| matches!(
            trak.mdia.minf.stbl.chunk_offset,
            Some(ChunkOffsetVariant::ChunkOffset(_))
        )));
        assert_eq!(samples, read(&file).unwrap().1);
    }

    #[test]
    fn a_fragmented_movie_is_refused() {
        let file = include_bytes!("../crates/isobmff/tests/fixtures/fragmented.mp4");
        assert_eq!(
            written(file).unwrap_err().to_string(),
            "a fragmented movie, whose movie fragments the moov does not declare"
        );
    }

    #[test]
    fn a_moov_with_an_offset_into_the_moov_is_refused() {
        let file = moov_last_with(|moov, moov_offset| {
            let stbl = &mut moov.trak[0].mdia.minf.stbl;
            if let Some(ChunkOffsetVariant::ChunkOffset(stco)) = &mut stbl.chunk_offset {
                stco.chunk_offsets[0] = u32::try_from(moov_offset + 8).unwrap();
            }
        });
        assert_eq!(
            written(&file).unwrap_err().to_string(),
            "offset 22603 points at nothing once moved"
        );
    }

    #[test]
    fn a_movie_with_a_track_whose_media_data_may_lie_in_another_file_is_refused() {
        let file = moov_last_with(|moov, _| {
            let mut elsewhere = DataEntryUrlBox::default();
            elsewhere.location = Some(Utf8String::from_slice(b"other.mp4"));
            moov.trak[0]
                .mdia
                .minf
                .dinf
                .dref
                .data_entries
                .push(DataEntry::Url(elsewhere));
        });
        assert_eq!(
            written(&file).unwrap_err().to_string(),
            "a track whose media data may lie elsewhere than in the file"
        );
    }

    #[test]
    fn a_movie_with_an_item_that_lies_in_the_file_is_refused() {
        for data_reference_index in [0, 1] {
            let file = moov_last_with(|moov, _| {
                let mut item = ItemLocation::default();
                item.item_id = 1;
                item.data_reference_index = data_reference_index;
                let mut iloc = ItemLocationBox::default();
                iloc.items = vec![item];
                let mut dinf = DataInformationBox::default();
                dinf.dref.data_entries = vec![DataEntry::Url(DataEntryUrlBox::default())];
                let mut meta = MetaBox::default();
                meta.iloc = Some(iloc);
                meta.dinf = Some(dinf);
                moov.meta = Some(meta);
            });
            assert_eq!(
                written(&file).unwrap_err().to_string(),
                "an item that lies in the file"
            );
        }
    }

    #[test]
    #[ignore = "needs ISOBMFF_CONFORMANCE_DIR and ffprobe"]
    fn the_conformance_movie_files_written_with_the_moov_first_read_as_the_same_samples_and_packets()
     {
        let directory =
            std::env::temp_dir().join(format!("isobmff-fast-start-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let (mut differences, mut refused) = (Vec::new(), Vec::new());
        let (mut moved, mut copied, mut widened, mut compared) = (0, 0, 0, 0);
        for file in conformance_files() {
            // Why skip these: a file the demuxer reports an error in, such as
            // one whose samples lie in another file, has none to compare.
            let Some((source_moov, samples)) = read(&file.bytes) else {
                continue;
            };
            let finished = match written(&file.bytes) {
                Ok(finished) => finished,
                Err(error) => {
                    refused.push(format!("{}: {error}", file.name));
                    continue;
                }
            };
            let types = top_level_box_types(&finished);
            let moov = types.iter().position(|box_type| box_type == "moov");
            let mdat = types.iter().position(|box_type| box_type == "mdat");
            if moov.is_none_or(|moov| mdat.is_some_and(|mdat| moov > mdat)) {
                differences.push(format!("{}: the moov is not first: {types:?}", file.name));
            }
            if finished == file.bytes {
                copied += 1;
                continue;
            }
            moved += 1;
            let Some((moov, read_back)) = read(&finished) else {
                differences.push(format!("{}: the demuxer cannot read it", file.name));
                continue;
            };
            if read_back != samples {
                differences.push(format!("{}: the samples differ", file.name));
            }
            let large = |moov: &MovieBox| {
                moov.trak
                    .iter()
                    .filter(|trak| {
                        matches!(
                            trak.mdia.minf.stbl.chunk_offset,
                            Some(ChunkOffsetVariant::ChunkLargeOffset(_))
                        )
                    })
                    .count()
            };
            if large(&moov) > large(&source_moov) {
                widened += 1;
            }
            // Why compare these with this crate's demuxer alone: ffprobe
            // refuses to read the source.
            if !ffprobe(&["-v", "error"], &file.path).status.success() {
                continue;
            }
            let path = directory.join(&file.name);
            std::fs::write(&path, &finished).unwrap();
            compared += 1;
            let (theirs, ours) = (
                ffprobe_packets(&[], &file.path),
                ffprobe_packets(&[], &path),
            );
            let presented = |file: &[u8], packets: &[FfprobePacket]| {
                packets
                    .iter()
                    .map(|packet| {
                        let start = usize::try_from(packet.pos).unwrap();
                        let end = start + usize::try_from(packet.size).unwrap();
                        (
                            packet.track_id,
                            packet.dts,
                            packet.pts,
                            packet.key,
                            packet.discard,
                            file.get(start..end).map(<[u8]>::to_vec),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            let same = presented(&file.bytes, &theirs) == presented(&finished, &ours);
            if !same {
                differences.push(format!(
                    "{}: ffprobe reads {} packets, {} from the source, or their bytes differ",
                    file.name,
                    ours.len(),
                    theirs.len()
                ));
            }
        }
        std::fs::remove_dir_all(&directory).unwrap();
        println!(
            "{moved} files moved, {copied} copied as they are, {compared} compared with ffprobe, {widened} with a chunk offset table widened; refused: {refused:#?}"
        );
        assert_eq!(differences, Vec::<String>::new());
        assert!(moved > 0);
    }

    #[test]
    fn an_output_that_already_exists_is_refused_and_the_input_is_left_as_it_was() {
        let directory =
            std::env::temp_dir().join(format!("isobmff-fast-start-output-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let (input, link, other) = (
            directory.join("input.mp4"),
            directory.join("link.mp4"),
            directory.join("other.mp4"),
        );
        std::fs::write(&input, MOOV_LAST).unwrap();
        std::fs::hard_link(&input, &link).unwrap();
        std::fs::write(&other, b"other").unwrap();
        for output in [&input, &link, &other] {
            let refused = run(&input, output).is_err();
            assert_eq!(
                (
                    refused,
                    std::fs::read(&input).unwrap(),
                    std::fs::read(&other).unwrap()
                ),
                (true, MOOV_LAST.to_vec(), b"other".to_vec())
            );
        }
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
