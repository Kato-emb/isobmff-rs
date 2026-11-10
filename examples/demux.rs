//! A file read with the demuxer, its samples counted by track
//!
//! The example drives [`Demuxer`] the way a caller that seeks does: it hands
//! over the file from where the demuxer asks, subscribes to every track once
//! the `moov` is read, and takes the bytes of each sample it lends.
//!
//! `cargo run --example demux -- <input>`

use std::collections::BTreeMap;
use std::error::Error;

use isobmff::demuxer::{self, Event};
use isobmff::{Demuxer, DemuxerConfig};

fn main() -> Result<(), Box<dyn Error>> {
    let input = std::env::args().nth(1).ok_or("usage: demux <input>")?;
    let (tracks, errors) = count_samples(&std::fs::read(input)?);
    for (track_id, (samples, length)) in tracks {
        println!("track {track_id}: {samples} samples, {length} bytes");
    }
    for error in errors {
        println!("error: {error}");
    }
    Ok(())
}

/// Reads `file` where the demuxer asks, every track subscribed to: returns the number of samples of each track and of their bytes, and what the demuxer could not read
fn count_samples(file: &[u8]) -> (BTreeMap<u32, (usize, usize)>, Vec<demuxer::Error>) {
    let mut demuxer = Demuxer::new(DemuxerConfig::default());
    let (mut tracks, mut errors) = (BTreeMap::<u32, (usize, usize)>::new(), Vec::new());
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
                Event::Error(error) => errors.push(error),
                _ => {}
            }
        }
        // Read the file where the demuxer asks, and end it where it ends
        let Some(required) = demuxer.required_input() else {
            break;
        };
        match usize::try_from(required.position)
            .ok()
            .and_then(|position| file.get(position..))
        {
            Some(bytes) if !bytes.is_empty() => demuxer.handle_input(required.position, bytes),
            _ => demuxer.finish(),
        }
    }
    (tracks, errors)
}

#[cfg(test)]
mod tests {
    use super::count_samples;

    #[test]
    fn every_sample_of_a_fixture_is_lent_whether_its_moov_comes_first_or_last() {
        for file in [
            include_bytes!("../crates/isobmff/tests/fixtures/movie_moov_first.mp4").as_slice(),
            include_bytes!("../crates/isobmff/tests/fixtures/movie_moov_last.mp4"),
        ] {
            assert_eq!(
                count_samples(file),
                ([(1, (12, 17_615)), (2, (58, 4_932))].into(), Vec::new())
            );
        }
    }
}
