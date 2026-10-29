//! Lists the boxes and the samples of a movie file or a segment as its bytes arrive in order, without seeking
//!
//! ```text
//! cargo run -p isobmff --example demux_stream -- < movie.mp4
//! curl -s https://example.com/movie.mp4 | cargo run -p isobmff --example demux_stream
//! ```
//!
//! The file is read from standard input a buffer at a time. Each buffer goes
//! to the demuxer again and again while it returns events, one box at file
//! level each. After each call, the samples whose bytes have arrived by then
//! come out of `poll_sample`, in the order of their locations. Each box and
//! each sample is printed as it comes out.
//!
//! The demuxer cannot tell where the file ends, so the caller calls `finish`
//! when standard input ends.
//!
//! A caller that cannot seek reads the samples of a file whose `moov` and
//! `moof`s each come before their media data: media data in a buffer passed
//! before the box that locates it is read is not kept, and `finish` fails.
//!
//! The demuxer holds the locations of every sample of a `moov` or a `moof` at
//! once, so their bytes together have to fit `DemuxLimits::with_held_bytes`.

use std::collections::BTreeMap;
use std::error::Error;
use std::io::{self, Read};

use isobmff::boxes::{
    FileTypeBox, MovieBox, MovieFragmentBox, MovieFragmentRandomAccessBox, SegmentIndexBox,
    SegmentTypeBox,
};
use isobmff::{BoxDefinition, DemuxEvent, Demuxer};

fn main() -> Result<(), Box<dyn Error>> {
    let mut demuxer = Demuxer::new();
    let mut stdin = io::stdin().lock();
    let mut buffer = vec![0; 64 * 1024];
    let mut offset = 0_u64;
    let mut gathered = BTreeMap::<u32, (u64, u64)>::new();

    loop {
        let read = stdin.read(&mut buffer)?;
        let piece = buffer.get(..read).ok_or("read past the buffer")?;
        if piece.is_empty() {
            break;
        }
        while let Some(event) = demuxer.handle_input(offset, piece)? {
            print_box(&event);
            take_samples(&mut demuxer, &mut gathered)?;
        }
        take_samples(&mut demuxer, &mut gathered)?;
        offset = offset.saturating_add(u64::try_from(read)?);
    }
    while let Some(event) = demuxer.finish()? {
        print_box(&event);
        take_samples(&mut demuxer, &mut gathered)?;
    }
    take_samples(&mut demuxer, &mut gathered)?;

    for (track_id, (samples, bytes)) in gathered {
        println!("track {track_id}: {samples} samples, {bytes} bytes");
    }
    Ok(())
}

/// Prints one line for the box at file level `event` reports: its type and where it lies
fn print_box(event: &DemuxEvent) {
    let (box_type, byte_range) = match event {
        DemuxEvent::FileType { byte_range, .. } => (FileTypeBox::BOX_TYPE, byte_range),
        DemuxEvent::SegmentType { byte_range, .. } => (SegmentTypeBox::BOX_TYPE, byte_range),
        DemuxEvent::Movie { byte_range, .. } => (MovieBox::BOX_TYPE, byte_range),
        DemuxEvent::MovieFragment { byte_range, .. } => (MovieFragmentBox::BOX_TYPE, byte_range),
        DemuxEvent::SegmentIndex { byte_range, .. } => (SegmentIndexBox::BOX_TYPE, byte_range),
        DemuxEvent::MovieFragmentRandomAccess { byte_range, .. } => {
            (MovieFragmentRandomAccessBox::BOX_TYPE, byte_range)
        }
        DemuxEvent::MediaData {
            header, byte_range, ..
        }
        | DemuxEvent::Other {
            header, byte_range, ..
        } => (header.box_type(), byte_range),
        _ => return,
    };
    let size = byte_range
        .length()
        .map_or_else(|| "to end of file".to_owned(), |length| length.to_string());
    println!("[{box_type}] offset={} size={size}", byte_range.offset());
}

/// Prints each sample `demuxer` has gathered, and counts them and their bytes by track
fn take_samples(
    demuxer: &mut Demuxer,
    gathered: &mut BTreeMap<u32, (u64, u64)>,
) -> Result<(), Box<dyn Error>> {
    while let Some(sample) = demuxer.poll_sample() {
        let properties = sample.properties();
        println!(
            "  sample of track {}: decoding time {}, {} bytes",
            properties.track_id,
            properties.decoding_time,
            sample.data().len()
        );
        let (samples, bytes) = gathered.entry(properties.track_id).or_default();
        *samples = samples.saturating_add(1);
        *bytes = bytes.saturating_add(u64::try_from(sample.data().len())?);
    }
    Ok(())
}
