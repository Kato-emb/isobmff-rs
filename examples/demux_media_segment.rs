//! Reads the samples a media segment carries, one line per sample
//!
//! The movie the segment continues is taken from its initialization segment.
//!
//! Usage: `cargo run -p isobmff-examples --example demux_media_segment -- <initialization.mp4> <segment.m4s>`

use core::error::Error;
use std::env;
use std::fs::File;

use isobmff::io::blocking::{FragmentedDemuxer, MediaSegmentDemuxer};

fn main() -> Result<(), Box<dyn Error>> {
    let usage = "usage: demux_media_segment <initialization.mp4> <segment.m4s>";
    let mut arguments = env::args().skip(1);
    let initialization_path = arguments.next().ok_or(usage)?;
    let segment_path = arguments.next().ok_or(usage)?;

    let mut initialization = FragmentedDemuxer::new(File::open(initialization_path)?)?;
    initialization.next().transpose()?;
    let movie = initialization
        .movie()
        .ok_or("the initialization segment carries no movie")?
        .clone();
    let demuxer = MediaSegmentDemuxer::new(File::open(segment_path)?, movie)?;

    let mut count: u64 = 0;
    for sample in demuxer {
        let sample = sample?;
        println!(
            "track={} time={} size={} sync={}",
            sample.track_id(),
            sample.decode_time(),
            sample.data().len(),
            !sample.sample_flags().sample_is_non_sync_sample(),
        );
        count = count.saturating_add(1);
    }
    println!("samples={count}");

    Ok(())
}
