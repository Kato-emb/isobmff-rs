//! Splits a fragmented movie file into an initialization segment and one media segment per movie fragment
//!
//! ```text
//! cargo run -p isobmff --example split_fragments -- movie.mp4 segments/
//! ```
//!
//! The initialization segment, `init.mp4`, holds the `ftyp` and the `moov`
//! (ISO/IEC 14496-12 A.10), and the `mdat` of the samples the `moov` itself
//! holds, if any, at the file offset its chunk offsets point to. Each media
//! segment, `segment-<n>.m4s`, opens with a `styp` carrying the brands of the
//! `ftyp` (§8.14.2) and holds one `moof` with the `mdat` that follows it
//! (A.11). Every other box, such as a `sidx` or an `mfra`, is skipped without
//! reading its payload. Concatenating `init.mp4` with the segments in order
//! gives a file that plays as the original does.
//!
//! The reader's events go to a writer for each segment. The `ftyp`, `moov`
//! and `moof` are gathered whole; a `mdat` is streamed, its payload going to
//! the segment a piece at a time as it is read, never held whole.

use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

use isobmff::framing::{BoxEvent, BoxReader, BoxWriter};
use isobmff::{BoxHeader, BoxType};

/// Longest `ftyp`, `moov` or `moof` read whole
const GATHER_LIMIT: u64 = 16 * 1024 * 1024;

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let (Some(path), Some(out_dir)) = (arguments.next(), arguments.next()) else {
        return Err("usage: split_fragments <file> <out_dir>".into());
    };
    let out_dir = Path::new(&out_dir);
    fs::create_dir_all(out_dir)?;

    let mut file = File::open(path)?;
    let mut reader = BoxReader::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut output = BufWriter::new(File::create(out_dir.join("init.mp4"))?);
    let mut writer = BoxWriter::new();
    let mut brands: Option<Vec<u8>> = None;
    let mut segment_count: u32 = 0;
    let mut copying = false;

    let (mut buffered_at, mut buffered) = (0_u64, 0_usize);

    while let Some(wanted) = reader.wanted_input() {
        let buffered_end = buffered_at.saturating_add(u64::try_from(buffered)?);
        if !(buffered_at..buffered_end).contains(&wanted.offset()) {
            file.seek(SeekFrom::Start(wanted.offset()))?;
            let at_most = wanted
                .length()
                .and_then(|length| usize::try_from(length).ok())
                .map_or(buffer.len(), |length| length.min(buffer.len()));
            buffered = file.read(buffer.get_mut(..at_most).ok_or("buffer too short")?)?;
            buffered_at = wanted.offset();
            if buffered == 0 {
                break;
            }
        }
        let input = buffer.get(..buffered).ok_or("read past the buffer")?;

        for event in reader.handle_input(buffered_at, input)? {
            if let BoxEvent::Header(header) = event {
                let box_type = header.box_type();
                copying = [b"ftyp", b"moov", b"moof", b"mdat"]
                    .iter()
                    .any(|copied| box_type == BoxType::compact(**copied));
                if !copying {
                    reader.skip()?;
                    continue;
                }
                if box_type == BoxType::compact(*b"moof") {
                    writer.finish()?;
                    segment_count = segment_count.saturating_add(1);
                    output.flush()?;
                    output = BufWriter::new(File::create(
                        out_dir.join(format!("segment-{segment_count}.m4s")),
                    )?);
                    writer = BoxWriter::new();
                    let brands = brands.as_deref().ok_or("a moof before the ftyp")?;
                    let segment_type = BoxHeader::with_payload_len(
                        BoxType::compact(*b"styp"),
                        u64::try_from(brands.len())?,
                    )
                    .ok_or("brands too long for a box")?;
                    for styp in [
                        BoxEvent::Header(segment_type),
                        BoxEvent::Payload(brands),
                        BoxEvent::End,
                    ] {
                        output.write_all(writer.handle_event(styp)?)?;
                    }
                }
                if box_type == BoxType::compact(*b"mdat") {
                    reader.stream()?;
                } else {
                    reader.gather(GATHER_LIMIT)?;
                }
                output.write_all(writer.handle_event(event)?)?;
                if segment_count == 0
                    && box_type == BoxType::compact(*b"mdat")
                    && writer.open_box().map(|written| written.offset())
                        != reader.open_box().map(|read| read.offset())
                {
                    return Err(
                        "the media data of the movie would move from where its chunk offsets point"
                            .into(),
                    );
                }
                continue;
            }
            if !copying {
                continue;
            }
            if let Some(raw_box) = reader.gathered_box() {
                if raw_box.header().box_type() == BoxType::compact(*b"ftyp") {
                    brands = Some(raw_box.payload().to_vec());
                }
                output.write_all(writer.handle_event(BoxEvent::Payload(raw_box.payload()))?)?;
            }
            output.write_all(writer.handle_event(event)?)?;
        }
    }

    let _events = reader.finish()?;
    writer.finish()?;
    output.flush()?;
    println!(
        "wrote init.mp4 and {segment_count} media segments to {}",
        out_dir.display()
    );

    Ok(())
}
