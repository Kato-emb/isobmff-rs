//! Writes the samples of a file the demuxer reads with the hybrid writer of `isobmff_examples`, a fragment for every `samples_per_fragment` samples
//!
//! `cargo run -p isobmff-examples --bin hybrid -- <input> <output> [samples_per_fragment]`
//!
//! It takes as many samples of a track, and of a fragment, as the muxers take
//! by default.

use isobmff::MuxerConfig;
use isobmff_examples::demux::read;
use isobmff_examples::hybrid::HybridWriter;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let usage = "usage: hybrid <input> <output> [samples_per_fragment]";
    let (input, output) = (
        arguments.next().ok_or(usage)?,
        arguments.next().ok_or(usage)?,
    );
    let samples_per_fragment = arguments
        .next()
        .map_or(Ok(30), |count| count.parse::<usize>())?;
    let source = read(&std::fs::read(input)?).ok_or("no ftyp or no moov")?;

    let mut written = Vec::new();
    let mut writer = HybridWriter::new(
        &source.file_type,
        source.movie,
        MuxerConfig::default().sample_count_limit,
        &mut written,
    )?;
    for (index, (properties, bytes)) in source.samples.iter().enumerate() {
        if index > 0 && index.checked_rem(samples_per_fragment) == Some(0) {
            writer.finish_fragment(&mut written)?;
        }
        writer.handle_sample(*properties, bytes)?;
    }
    let header = writer.finish(&mut written)?;
    let free = usize::try_from(header.offset)?;
    written
        .get_mut(free..)
        .and_then(|rest| rest.get_mut(..header.bytes.len()))
        .ok_or("the free lies past the file")?
        .copy_from_slice(&header.bytes);
    std::fs::write(output, written)?;
    Ok(())
}
