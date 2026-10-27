//! Prints the box structure of a file, reading its headers and skipping its media data
//!
//! ```text
//! cargo run -p isobmff --example dump_boxes -- movie.mp4
//! ```
//!
//! Each top-level box is read whole, up to a limit, and the boxes a container
//! holds are listed under it. A `mdat`, a box past the limit and a box that
//! runs to the end of the file are listed by their header alone: the reader
//! then asks for the bytes past the box, the caller seeks the file there, and the
//! payload is never read.

use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use isobmff::framing::{BoxEvent, BoxReader};
use isobmff::{BoxHeader, BoxType, Boxes, ByteRange, RawBox};

/// Longest payload read whole; a box past it is listed by its header alone
const GATHER_LIMIT: u64 = 16 * 1024 * 1024;

/// Boxes whose payload is a run of boxes (ISO/IEC 14496-12 §6.2 Table 2), listed with their children
const CONTAINERS: [&[u8; 4]; 14] = [
    b"moov", b"trak", b"edts", b"mdia", b"minf", b"dinf", b"stbl", b"mvex", b"udta", b"moof",
    b"traf", b"mfra", b"sinf", b"schi",
];

fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args().nth(1).ok_or("usage: dump_boxes <file>")?;
    let mut file = File::open(path)?;
    let mut reader = BoxReader::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut gathering: Option<ByteRange> = None;

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
                    if header.box_type() != BoxType::compact(*b"mdat")
                        && header
                            .payload_len()
                            .is_some_and(|payload_len| payload_len <= GATHER_LIMIT)
                    {
                        gathering = Some(open.byte_range());
                        open.gather(GATHER_LIMIT)?;
                    } else {
                        print_box(header, open.byte_range(), 0, " (not read)");
                        open.skip();
                    }
                }
                BoxEvent::Gathered(raw_box) => {
                    let byte_range = gathering
                        .take()
                        .ok_or("a box was gathered that was never opened")?;
                    print_tree(raw_box, byte_range.offset(), 0);
                }
                BoxEvent::Payload(_) | BoxEvent::End => {}
                _ => {}
            }
        }
    }

    for event in reader.finish()? {
        event?;
    }

    Ok(())
}

/// Prints `raw_box` at `offset`, and the boxes it holds if it is a container
fn print_tree(raw_box: RawBox<'_>, offset: u64, depth: usize) {
    let header = raw_box.header();
    print_box(
        header,
        ByteRange::new(offset, header.size().total_bytes()),
        depth,
        "",
    );
    if !CONTAINERS.contains(&header.box_type().four_cc().as_bytes()) {
        return;
    }

    let mut child_offset = offset.saturating_add(header.encoded_len());
    for child in Boxes::new(raw_box.payload()) {
        match child {
            Ok(child) => {
                print_tree(child, child_offset, depth.saturating_add(1));
                child_offset = child_offset
                    .saturating_add(child.header().size().total_bytes().unwrap_or(u64::MAX));
            }
            Err(error) => {
                println!(
                    "{:indent$}  ! {error}",
                    "",
                    indent = depth.saturating_mul(2)
                );
                break;
            }
        }
    }
}

/// Prints one line for a box: its type, where it lies, and how long it is
fn print_box(header: BoxHeader, byte_range: ByteRange, depth: usize, note: &str) {
    let size = byte_range
        .length()
        .map_or_else(|| "to end of file".to_owned(), |length| length.to_string());
    println!(
        "{:indent$}[{}] offset={} size={size}{note}",
        "",
        header.box_type(),
        byte_range.offset(),
        indent = depth.saturating_mul(2)
    );
}
