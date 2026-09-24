//! Rewrites a fragmented MP4 file as a non-fragmented one, carrying every track over
//!
//! The movie header and the tracks of the source are kept, its `mvex` and every other box of the
//! movie dropped, and the writer fills the sample tables of each track in from the samples; the
//! durations the movie declares are carried over as they stand, which a movie whose samples all
//! lie in fragments may leave at zero. A chunk opens wherever the samples pass to another track or
//! another sample description.
//!
//! Usage: `cargo run -p isobmff-examples --example remux_to_non_fragmented -- <in.mp4> <out.mp4>`

use core::error::Error;
use std::env;
use std::fs::File;
use std::io::BufWriter;

use isobmff::boxes::MovieBox;
use isobmff::io::blocking::{FragmentedDemuxer, NonFragmentedMuxer};

fn main() -> Result<(), Box<dyn Error>> {
    let usage = "usage: remux_to_non_fragmented <in.mp4> <out.mp4>";
    let mut arguments = env::args().skip(1);
    let input = arguments.next().ok_or(usage)?;
    let output = arguments.next().ok_or(usage)?;

    let mut demuxer = FragmentedDemuxer::new(File::open(input)?)?;
    let first = demuxer.next().transpose()?;
    let source = demuxer.movie().ok_or("the file carries no movie")?;
    let movie = MovieBox::new(source.mvhd().clone(), source.trak().to_vec(), None)
        .ok_or("the movie declares no track")?;

    let mut muxer = NonFragmentedMuxer::new(BufWriter::new(File::create(output)?));
    if let Some(file_type) = demuxer.file_type() {
        muxer.handle_file_type(file_type.clone())?;
    }
    muxer.handle_movie(movie)?;
    let mut chunk = None;
    for sample in first.into_iter().map(Ok).chain(demuxer) {
        let sample = sample?;
        let described_by = Some((sample.track_id(), sample.sample_description_index()));
        if chunk != described_by {
            chunk = described_by;
            muxer.begin_chunk()?;
        }
        muxer.handle_sample(sample)?;
    }
    muxer.finish()?;

    Ok(())
}
