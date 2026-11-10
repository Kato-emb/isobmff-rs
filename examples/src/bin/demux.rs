//! Prints the samples of each track of a file the demuxer reads
//!
//! `cargo run -p isobmff-examples --bin demux -- <input>`

use std::collections::BTreeMap;

use isobmff_examples::demux::read;

#[expect(clippy::print_stdout, reason = "a program that prints what it read")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = std::env::args().nth(1).ok_or("usage: demux <input>")?;
    let source = read(&std::fs::read(input)?).ok_or("no ftyp or no moov")?;
    let mut tracks = BTreeMap::<u32, (usize, usize)>::new();
    for (properties, bytes) in &source.samples {
        let (samples, length) = tracks.entry(properties.track_id).or_default();
        *samples = samples.saturating_add(1);
        *length = length.saturating_add(bytes.len());
    }
    for (track_id, (samples, length)) in tracks {
        println!("track {track_id}: {samples} samples, {length} bytes");
    }
    for error in &source.errors {
        println!("error: {error}");
    }
    Ok(())
}
