//! A file read with the demuxer where it asks, its samples counted by track
//!
//! The example drives [`Demuxer`] the way a caller that seeks does: it asks
//! [`required_input`](Demuxer::required_input) where to read, seeks there,
//! hands over what one read of a buffer returns, and ends the input where the
//! file ends. It subscribes to every track once the `moov` is read, and counts
//! the samples the demuxer lends. Memory is bounded by the limits of
//! [`DemuxerConfig`] and the buffer, whatever the length of the file.
//!
//! `cargo run --example demux -- <input>`

use std::collections::BTreeMap;
use std::error::Error;
use std::io::{Read, Seek, SeekFrom, Write};

use isobmff::demuxer::Event;
use isobmff::{Demuxer, DemuxerConfig};

fn main() -> Result<(), Box<dyn Error>> {
    let input = std::env::args().nth(1).ok_or("usage: demux <input>")?;
    demux(std::fs::File::open(input)?, std::io::stdout().lock())
}

/// Reads `input` where the demuxer asks, every track subscribed to, and writes to `summary` how many samples each track has and how many bytes they hold, and what the demuxer could not read
fn demux(mut input: impl Read + Seek, mut summary: impl Write) -> Result<(), Box<dyn Error>> {
    let mut demuxer = Demuxer::new(DemuxerConfig::default());
    let mut buffer = vec![0; 64 * 1024];
    let mut tracks = BTreeMap::<u32, (usize, usize)>::new();
    loop {
        // Report what the bytes handed over hold
        while let Some(event) = demuxer.poll_event() {
            match event {
                Event::Movie(moov) => {
                    for trak in &moov.trak {
                        demuxer.subscribe_track(trak.tkhd.track_id);
                    }
                }
                Event::Sample(sample) => {
                    let (samples, length) = tracks.entry(sample.properties.track_id).or_default();
                    *samples += 1;
                    *length += sample.bytes.len();
                }
                Event::Error(error) => writeln!(summary, "error: {error}")?,
                _ => {}
            }
        }
        // Read the file where the demuxer asks, and end it where it ends
        let Some(required) = demuxer.required_input() else {
            break;
        };
        input.seek(SeekFrom::Start(required.position))?;
        match input.read(&mut buffer)? {
            0 => demuxer.finish(),
            read => demuxer.handle_input(required.position, &buffer[..read]),
        }
    }
    for (track_id, (samples, length)) in tracks {
        writeln!(
            summary,
            "track {track_id}: {samples} samples, {length} bytes"
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::demux;

    #[test]
    fn every_sample_of_a_fixture_is_lent_whether_its_moov_comes_first_or_last() {
        for file in [
            include_bytes!("../crates/isobmff/tests/fixtures/movie_moov_first.mp4").as_slice(),
            include_bytes!("../crates/isobmff/tests/fixtures/movie_moov_last.mp4"),
        ] {
            let mut summary = Vec::new();
            demux(Cursor::new(file), &mut summary).unwrap();
            assert_eq!(
                String::from_utf8(summary).unwrap(),
                "track 1: 12 samples, 17615 bytes\ntrack 2: 58 samples, 4932 bytes\n"
            );
        }
    }
}
