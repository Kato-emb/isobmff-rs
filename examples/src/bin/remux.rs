//! Writes the samples of a file the demuxer reads into a movie file, or a fragmented movie file with `--fragmented`
//!
//! `cargo run -p isobmff-examples --bin remux -- <input> <output> [--fragmented]`

use isobmff_examples::demux::read;
use isobmff_examples::remux::{fragmented_movie_file, movie_file};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let usage = "usage: remux <input> <output> [--fragmented]";
    let (input, output) = (
        arguments.next().ok_or(usage)?,
        arguments.next().ok_or(usage)?,
    );
    let fragmented = arguments.next().as_deref() == Some("--fragmented");
    let source = read(&std::fs::read(input)?).ok_or("no ftyp or no moov")?;
    let written = if fragmented {
        fragmented_movie_file(&source)?
    } else {
        movie_file(&source)?
    };
    std::fs::write(output, written)?;
    Ok(())
}
