//! Lists the boxes and the samples of a movie file or a segment as a demuxer reads them, seeking to the bytes it wants
//!
//! ```text
//! cargo run --example demux -- movie.mp4
//! ```
//!
//! The demuxer names the file offset it wants next, and the caller passes the
//! bytes of the file from there. For each box at file level, the demuxer
//! returns an event. It returns each sample once the bytes of the sample have
//! arrived, and the caller takes the samples after each call, before it
//! looks at the event or the failure. When no byte is left at the wanted
//! offset, the file has ended, and the caller tells the demuxer with
//! `finish`.
//!
//! The media data of a track may be in another file, which an entry of its
//! data reference names (ISO/IEC 14496-12 §8.7.2). The demuxer locates those
//! samples but does not gather them. This example counts them and names the
//! files they are in.

use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};

use isobmff::boxes::DataEntry;
use isobmff::{DemuxEvent, Demuxer};

fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args().nth(1).ok_or("usage: demux <file>")?;
    let mut file = BufReader::new(File::open(path)?);
    let mut demuxer = Demuxer::new();

    while let Some(wanted) = demuxer.wanted_input() {
        file.seek(SeekFrom::Start(wanted.offset()))?;
        let event = match file.fill_buf()? {
            [] => demuxer.finish(),
            input => demuxer.handle_input(wanted.offset(), input),
        };
        while let Some(sample) = demuxer.poll_sample() {
            let properties = sample.properties();
            println!(
                "  sample of track {}: decoding time {}, {} bytes",
                properties.track_id,
                properties.decoding_time,
                sample.data().len()
            );
        }
        if let Some(event) = event? {
            print_event(&demuxer, &event);
        }
    }

    for data_reference in demuxer.data_references() {
        let entry = data_reference.entry();
        if entry.is_in_same_file() {
            continue;
        }
        let named = if let DataEntry::Url(url) = entry
            && let Some(location) = url.location.as_ref().and_then(|location| location.as_str())
        {
            location.to_owned()
        } else {
            format!("{entry:?}")
        };
        println!(
            "track {}: samples in {named}, not read",
            data_reference.track_id()
        );
    }
    Ok(())
}

/// Prints one line for the box at file level `event` reports, and one more for the samples a `moov` or a `moof` locates
fn print_event(demuxer: &Demuxer, event: &DemuxEvent) {
    let (box_type, byte_range) = match event {
        DemuxEvent::FileType { byte_range, .. } => ("ftyp".to_owned(), byte_range),
        DemuxEvent::SegmentType { byte_range, .. } => ("styp".to_owned(), byte_range),
        DemuxEvent::Movie { byte_range, .. } => ("moov".to_owned(), byte_range),
        DemuxEvent::MovieFragment { byte_range, .. } => ("moof".to_owned(), byte_range),
        DemuxEvent::SegmentIndex { byte_range, .. } => ("sidx".to_owned(), byte_range),
        DemuxEvent::MovieFragmentRandomAccess { byte_range, .. } => ("mfra".to_owned(), byte_range),
        DemuxEvent::MediaData {
            header, byte_range, ..
        }
        | DemuxEvent::Other {
            header, byte_range, ..
        } => (header.box_type().to_string(), byte_range),
        _ => return,
    };
    let size = byte_range
        .length()
        .map_or_else(|| "to end of file".to_owned(), |length| length.to_string());
    println!("[{box_type}] offset={} size={size}", byte_range.offset());

    if let DemuxEvent::Movie { samples, .. } | DemuxEvent::MovieFragment { samples, .. } = event {
        let elsewhere = samples
            .clone()
            .filter(|location| {
                demuxer
                    .data_reference(location.data_reference())
                    .is_some_and(|data_reference| !data_reference.entry().is_in_same_file())
            })
            .count();
        println!(
            "  locates {} samples, {elsewhere} of them in other files",
            samples.len()
        );
    }
}
