//! Gathers the samples of a movie file or a segment out of its bytes as they arrive in order, without seeking
//!
//! ```text
//! cargo run -p isobmff --example demux_stream -- < movie.mp4
//! curl -s https://example.com/movie.mp4 | cargo run -p isobmff --example demux_stream
//! ```
//!
//! The file is read from standard input a buffer at a time. Each buffer goes
//! to the demuxer again and again while it returns events; the samples whose
//! bytes have arrived by then come out of `poll_sample`, and are counted.
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

use isobmff::Demuxer;

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
        while demuxer.handle_input(offset, piece)?.is_some() {}
        count(&mut demuxer, &mut gathered)?;
        offset = offset.saturating_add(u64::try_from(read)?);
    }
    while demuxer.finish()?.is_some() {}
    count(&mut demuxer, &mut gathered)?;

    for (track_id, (samples, bytes)) in gathered {
        println!("track {track_id}: {samples} samples, {bytes} bytes");
    }
    Ok(())
}

/// Counts the samples `demuxer` has gathered, and their bytes, by track
fn count(
    demuxer: &mut Demuxer,
    gathered: &mut BTreeMap<u32, (u64, u64)>,
) -> Result<(), Box<dyn Error>> {
    while let Some(sample) = demuxer.poll_sample() {
        let (samples, bytes) = gathered.entry(sample.properties().track_id).or_default();
        *samples = samples.saturating_add(1);
        *bytes = bytes.saturating_add(u64::try_from(sample.data().len())?);
    }
    Ok(())
}
