//! Gathers the samples of a movie file or a segment out of its bytes as they arrive in order, without seeking
//!
//! ```text
//! cargo run -p isobmff --example demux_stream -- < movie.mp4
//! curl -s https://example.com/movie.mp4 | cargo run -p isobmff --example demux_stream
//! ```
//!
//! The file is read from standard input a buffer at a time, and each buffer
//! is passed to the demuxer until it wants bytes past it. Each `mdat` is
//! streamed, and its pieces go to a `MediaDataReader`, which holds the
//! locations of the samples the `moov` and the `moof`s declare and gathers
//! each sample out of them. Samples are counted as they are gathered.
//!
//! The demuxer cannot tell where the file ends, so the caller calls `finish`
//! when standard input ends.
//!
//! A caller that cannot seek reads a fragmented movie file, or a file whose
//! `moov` comes before its media data: media data that goes by before the
//! `moov` is not kept, and `finish` of the `MediaDataReader` fails.
//!
//! The locations a `MediaDataReader` holds count toward its limit by the bytes
//! they name, so every sample of one `moov` or `moof` together has to fit
//! `held_bytes`.

use std::collections::BTreeMap;
use std::error::Error;
use std::io::{self, Read};

use isobmff::media_data::MediaDataReader;
use isobmff::{DemuxEvent, DemuxLimits, Demuxer};

fn main() -> Result<(), Box<dyn Error>> {
    let limits = DemuxLimits::new();
    let mut demuxer = Demuxer::with_limits(limits);
    let mut media_data = MediaDataReader::new(limits.media_data());
    let mut stdin = io::stdin().lock();
    let mut buffer = vec![0; 64 * 1024];
    let mut offset = 0_u64;
    let mut gathered = BTreeMap::<u32, (u64, u64)>::new();

    loop {
        let read = stdin.read(&mut buffer)?;
        let piece = buffer.get(..read).ok_or("read past the buffer")?;
        let at_end = piece.is_empty();
        let end = offset.saturating_add(u64::try_from(read)?);
        while demuxer
            .wanted_input()
            .is_some_and(|wanted| at_end || wanted.offset() < end)
        {
            let event = match piece {
                [] => demuxer.finish()?,
                piece => demuxer.handle_input(offset, piece)?,
            };
            match event {
                Some(
                    DemuxEvent::Movie { samples, .. } | DemuxEvent::MovieFragment { samples, .. },
                ) => media_data.handle_sample_locations(samples)?,
                Some(DemuxEvent::MediaData { .. }) => demuxer.stream()?,
                Some(DemuxEvent::MediaDataPayload { offset, bytes, .. }) => {
                    media_data.handle_input(offset, bytes);
                }
                _ => {}
            }
            while let Some(sample) = media_data.poll_sample() {
                let (samples, bytes) = gathered.entry(sample.properties().track_id).or_default();
                *samples = samples.saturating_add(1);
                *bytes = bytes.saturating_add(u64::try_from(sample.data().len())?);
            }
        }
        if at_end {
            break;
        }
        offset = end;
    }
    media_data.finish()?;

    for (track_id, (samples, bytes)) in gathered {
        println!("track {track_id}: {samples} samples, {bytes} bytes");
    }
    Ok(())
}
