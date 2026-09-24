//! Reads the samples of a media segment from the subsegment its `sidx` names for a time on
//!
//! The segment is read on until one of its segment indexes covers the time, and the reading
//! resumes at the first byte of the subsegment that index names — where that is a reference to
//! another index, at that index. The time is given in milliseconds of presentation time; a
//! segment none of whose indexes covers it is refused.
//!
//! Usage: `cargo run -p isobmff-examples --example seek_media_segment -- <initialization.mp4> <segment.m4s> <milliseconds>`

use core::error::Error;
use std::env;
use std::fs::File;

use isobmff::io::blocking::{FragmentedDemuxer, MediaSegmentDemuxer};

fn main() -> Result<(), Box<dyn Error>> {
    let usage = "usage: seek_media_segment <initialization.mp4> <segment.m4s> <milliseconds>";
    let mut arguments = env::args().skip(1);
    let initialization_path = arguments.next().ok_or(usage)?;
    let segment_path = arguments.next().ok_or(usage)?;
    let milliseconds: u64 = arguments.next().ok_or(usage)?.parse()?;

    let mut initialization = FragmentedDemuxer::new(File::open(initialization_path)?)?;
    initialization.next().transpose()?;
    let movie = initialization
        .movie()
        .ok_or("the initialization segment carries no movie")?
        .clone();
    let mut demuxer = MediaSegmentDemuxer::new(File::open(segment_path)?, movie)?;

    let subsegment_start = loop {
        let covering = demuxer.segment_indexes().iter().find_map(|index| {
            let time = milliseconds.checked_mul(u64::from(index.timescale()))? / 1_000;
            index.subsegment_at(time)
        });
        if let Some(subsegment) = covering {
            break subsegment.extent().start;
        }
        demuxer.next().ok_or("no segment index covers the time")??;
    };
    demuxer.resume_at(subsegment_start)?;

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
