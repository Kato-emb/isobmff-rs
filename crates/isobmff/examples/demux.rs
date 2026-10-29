//! Lists the boxes and the samples of a movie file or a segment, reading its structure-data and seeking past its media data
//!
//! ```text
//! cargo run -p isobmff --example demux -- movie.mp4
//! ```
//!
//! The demuxer names the bytes it wants next. The caller seeks the file there
//! and reads a buffer of it, then passes that buffer again for as long as the
//! demuxer returns events, reading another only once the demuxer wants bytes
//! past it. A read that returns no byte is the end of the file, which the
//! caller tells the demuxer with `finish`.
//!
//! The demuxer skips each `mdat`: its payload is never read while demuxing.
//! The bytes of a sample are read afterwards from the file, at the byte range
//! of its location, as a player reads them when it needs them.

use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use isobmff::{ByteRange, DemuxEvent, Demuxer, SampleLocation};

/// Samples listed per track
const LISTED_SAMPLES: usize = 3;

fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args().nth(1).ok_or("usage: demux <file>")?;
    let mut file = File::open(path)?;
    let mut demuxer = Demuxer::new();
    let mut buffer = vec![0; 64 * 1024];
    let (mut buffered_at, mut buffered) = (0_u64, 0_usize);
    let mut locations = Vec::new();

    while let Some(wanted) = demuxer.wanted_input() {
        let buffered_end = buffered_at.saturating_add(u64::try_from(buffered)?);
        if !(buffered_at..buffered_end).contains(&wanted.offset()) {
            file.seek(SeekFrom::Start(wanted.offset()))?;
            buffered = file.read(&mut buffer)?;
            buffered_at = wanted.offset();
        }
        let event = match buffer.get(..buffered).ok_or("read past the buffer")? {
            [] => demuxer.finish()?,
            input => demuxer.handle_input(buffered_at, input)?,
        };
        if let Some(event) = event {
            report(&demuxer, event, &mut locations)?;
        }
    }

    list_samples(&mut file, &locations)
}

/// Prints the box `event` reports, and keeps the locations of the samples it declares
fn report(
    demuxer: &Demuxer,
    event: DemuxEvent,
    locations: &mut Vec<SampleLocation>,
) -> Result<(), Box<dyn Error>> {
    match event {
        DemuxEvent::FileType { byte_range, .. } => {
            let ftyp = demuxer.file_type().ok_or("ftyp without its value")?;
            print_box(
                "ftyp",
                byte_range,
                &format!("major brand {}", ftyp.major_brand),
            );
        }
        DemuxEvent::SegmentType { byte_range, .. } => {
            let styp = demuxer.segment_type().ok_or("styp without its value")?;
            print_box(
                "styp",
                byte_range,
                &format!("major brand {}", styp.major_brand),
            );
        }
        DemuxEvent::Movie {
            byte_range,
            samples,
            ..
        } => {
            let tracks = demuxer.movie().map_or(0, |movie| movie.trak.len());
            print_box(
                "moov",
                byte_range,
                &format!("{tracks} tracks, {} samples", samples.len()),
            );
            locations.extend(samples);
        }
        DemuxEvent::MovieFragment {
            movie_fragment,
            byte_range,
            samples,
            ..
        } => {
            print_box(
                "moof",
                byte_range,
                &format!(
                    "sequence number {}, {} samples",
                    movie_fragment.mfhd.sequence_number,
                    samples.len()
                ),
            );
            locations.extend(samples);
        }
        DemuxEvent::SegmentIndex { byte_range, .. } => {
            let subsegments = demuxer
                .segment_indexes()
                .iter()
                .find(|index| index.byte_range().start == byte_range.offset())
                .map_or(0, |index| index.subsegments().len());
            print_box("sidx", byte_range, &format!("{subsegments} subsegments"));
        }
        DemuxEvent::MovieFragmentRandomAccess { byte_range, .. } => {
            let entries = demuxer
                .movie_fragment_random_access()
                .iter()
                .flat_map(|mfra| &mfra.tfra)
                .map(|tfra| tfra.entries.len())
                .sum::<usize>();
            print_box("mfra", byte_range, &format!("{entries} entries"));
        }
        DemuxEvent::MediaData {
            header, byte_range, ..
        }
        | DemuxEvent::Other {
            header, byte_range, ..
        } => {
            print_box(&header.box_type().to_string(), byte_range, "skipped");
        }
        _ => {}
    }
    Ok(())
}

/// Prints one line for a box at file level: its type, where it lies, and what was read of it
fn print_box(box_type: &str, byte_range: ByteRange, note: &str) {
    let size = byte_range
        .length()
        .map_or_else(|| "to end of file".to_owned(), |length| length.to_string());
    println!(
        "[{box_type}] offset={} size={size}: {note}",
        byte_range.offset()
    );
}

/// Prints how many samples each track has, and the first few with the first bytes of each, read from `file` at its byte range
fn list_samples(file: &mut File, locations: &[SampleLocation]) -> Result<(), Box<dyn Error>> {
    let mut by_track = BTreeMap::<u32, Vec<&SampleLocation>>::new();
    for location in locations {
        by_track
            .entry(location.properties().track_id)
            .or_default()
            .push(location);
    }
    for (track_id, samples) in by_track {
        println!("track {track_id}: {} samples", samples.len());
        for location in samples.iter().take(LISTED_SAMPLES) {
            let byte_range = location.byte_range();
            let mut data =
                vec![0; usize::try_from(byte_range.end.saturating_sub(byte_range.start))?];
            file.seek(SeekFrom::Start(byte_range.start))?;
            file.read_exact(&mut data)?;
            let head: Vec<_> = data
                .iter()
                .take(8)
                .map(|byte| format!("{byte:02x}"))
                .collect();
            println!(
                "  decoding time {} at {byte_range:?}: {} ...",
                location.properties().decoding_time,
                head.join(" ")
            );
        }
    }
    Ok(())
}
