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
//! The `ftyp`, `moov` and `moof` are read whole and written back. A `mdat` is
//! streamed: its payload goes to the segment as it is read, a piece at a time.

use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
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
    let mut output = File::create(out_dir.join("init.mp4"))?;
    let mut output_len: u64 = 0;
    let mut brands: Option<Vec<u8>> = None;
    let mut segment_count: u32 = 0;

    while let Some(wanted) = reader.wanted_input() {
        file.seek(SeekFrom::Start(wanted.offset()))?;
        let at_most = wanted
            .length()
            .and_then(|length| usize::try_from(length).ok())
            .map_or(buffer.len(), |length| length.min(buffer.len()));
        let read = file.read(buffer.get_mut(..at_most).ok_or("buffer too short")?)?;
        if read == 0 {
            break;
        }
        let input = buffer.get(..read).ok_or("read past the buffer")?;

        for event in reader.handle_input(wanted.offset(), input)? {
            match event? {
                BoxEvent::Header(open) => {
                    let header = open.header();
                    let box_type = header.box_type();
                    if box_type == BoxType::compact(*b"mdat") {
                        if segment_count == 0 && open.byte_range().offset() != output_len {
                            return Err("the media data of the movie would move from where its chunk offsets point".into());
                        }
                        let mut encoded = [0; 32];
                        header.encode(&mut encoded)?;
                        let header_len = usize::try_from(header.encoded_len())?;
                        output.write_all(encoded.get(..header_len).ok_or("header too long")?)?;
                        output_len = output_len.saturating_add(header.encoded_len());
                        open.stream();
                    } else if [b"ftyp", b"moov", b"moof"]
                        .iter()
                        .any(|whole| box_type == BoxType::compact(**whole))
                    {
                        open.gather(GATHER_LIMIT)?;
                    } else {
                        open.skip();
                    }
                }
                BoxEvent::Gathered(raw_box) => {
                    let box_type = raw_box.header().box_type();
                    let mut writer = BoxWriter::new();
                    if box_type == BoxType::compact(*b"ftyp") {
                        brands = Some(raw_box.payload().to_vec());
                    } else if box_type == BoxType::compact(*b"moof") {
                        let brands = brands.as_deref().ok_or("a moof before the ftyp")?;
                        let segment_type = BoxHeader::with_payload_len(
                            BoxType::compact(*b"styp"),
                            u64::try_from(brands.len())?,
                        )
                        .ok_or("brands too long for a box")?;
                        segment_count = segment_count.saturating_add(1);
                        output =
                            File::create(out_dir.join(format!("segment-{segment_count}.m4s")))?;
                        output_len = 0;
                        writer.handle_box(segment_type, [brands])?;
                    }
                    writer.handle_box(raw_box.header(), [raw_box.payload()])?;
                    while let Some(bytes) = writer.poll_output() {
                        output.write_all(&bytes)?;
                        output_len = output_len.saturating_add(u64::try_from(bytes.len())?);
                    }
                }
                BoxEvent::Payload(piece) => {
                    output.write_all(piece)?;
                    output_len = output_len.saturating_add(u64::try_from(piece.len())?);
                }
                BoxEvent::End => {}
                _ => {}
            }
        }
    }

    for event in reader.finish()? {
        event?;
    }
    println!(
        "wrote init.mp4 and {segment_count} media segments to {}",
        out_dir.display()
    );

    Ok(())
}
