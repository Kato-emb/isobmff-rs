//! Prints the tree of boxes an ISO base media file is formed as, one line per box
//!
//! Usage: `cargo run -p isobmff-examples --example dump_boxes -- <in.mp4>`

use core::error::Error;
use std::env;
use std::fs::File;
use std::io::Read;

use isobmff::core::{BoxHeader, BoxType, boxes};
use isobmff::sequence::{BoxEvent, BoxReader};

const CONTAINERS: [BoxType; 12] = [
    BoxType::compact(*b"moov"),
    BoxType::compact(*b"trak"),
    BoxType::compact(*b"edts"),
    BoxType::compact(*b"mdia"),
    BoxType::compact(*b"minf"),
    BoxType::compact(*b"dinf"),
    BoxType::compact(*b"stbl"),
    BoxType::compact(*b"mvex"),
    BoxType::compact(*b"moof"),
    BoxType::compact(*b"traf"),
    BoxType::compact(*b"mfra"),
    BoxType::compact(*b"udta"),
];

fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args().nth(1).ok_or("usage: dump_boxes <in.mp4>")?;
    let mut file = File::open(path)?;

    let mut reader = BoxReader::new();
    let mut cut = vec![0; 64 * 1024];
    let mut container: Option<(u64, Vec<u8>)> = None;
    loop {
        let read = file.read(&mut cut)?;
        if read == 0 {
            reader.finish()?;
        } else {
            reader.handle_input(cut.get(..read).unwrap_or_default())?;
        }
        while let Some(event) = reader.poll_event() {
            match event {
                BoxEvent::Header(header) => {
                    let extent = reader
                        .event_extent()
                        .ok_or("the event taken has no extent")?;
                    print_box(header, extent.start, 0);
                    container = CONTAINERS
                        .contains(&header.box_type())
                        .then(|| (extent.end, Vec::new()));
                }
                BoxEvent::Payload(payload) => {
                    if let Some((_, children)) = &mut container {
                        children.extend(payload);
                    }
                }
                BoxEvent::End => {
                    if let Some((offset, children)) = container.take() {
                        print_children(&children, offset, 1)?;
                    }
                }
                _ => {}
            }
        }
        if read == 0 {
            return Ok(());
        }
    }
}

/// Prints the boxes laid end to end in `payload`, the first starting at `offset` of the file
fn print_children(payload: &[u8], mut offset: u64, depth: usize) -> Result<(), Box<dyn Error>> {
    for child in boxes(payload) {
        let child = child?;
        let header = child.header();
        print_box(header, offset, depth);
        let payload_offset = offset
            .checked_add(u64::try_from(header.encoded_len())?)
            .ok_or("a box ends past u64::MAX")?;
        if CONTAINERS.contains(&header.box_type()) {
            print_children(child.payload(), payload_offset, depth.saturating_add(1))?;
        }
        offset = payload_offset
            .checked_add(u64::try_from(child.payload().len())?)
            .ok_or("a box ends past u64::MAX")?;
    }
    Ok(())
}

/// Prints one box as `<type> size=<size> offset=<offset>`, indented two spaces per level of depth
fn print_box(header: BoxHeader, offset: u64, depth: usize) {
    let indent = "  ".repeat(depth);
    let box_type = header.box_type();
    match header.size().total_bytes() {
        Some(size) => println!("{indent}{box_type} size={size} offset={offset}"),
        None => println!("{indent}{box_type} size=to-end-of-file offset={offset}"),
    }
}
