//! Reads the samples a media segment carries, one line per sample
//!
//! The movie the segment continues is taken from its initialization segment.
//!
//! Usage: `cargo run -p isobmff-examples --example demux_media_segment -- <initialization.mp4> <segment.m4s>`

use core::error::Error;
use std::env;
use std::fs::File;

use isobmff::io::blocking::Source;
use isobmff::structure::{FragmentedDemuxFsm, MediaSegmentDemuxFsm};

fn main() -> Result<(), Box<dyn Error>> {
    let usage = "usage: demux_media_segment <initialization.mp4> <segment.m4s>";
    let mut arguments = env::args().skip(1);
    let initialization_path = arguments.next().ok_or(usage)?;
    let segment_path = arguments.next().ok_or(usage)?;

    let mut initialization = Source::new(File::open(initialization_path)?)?;
    let mut initialization_fsm = FragmentedDemuxFsm::new();
    let movie = loop {
        if let Some(movie) = initialization_fsm.movie() {
            break movie.clone();
        }
        let wanted = initialization_fsm
            .wanted_input()
            .ok_or("the initialization segment carries no movie")?;
        let bytes = initialization.read_at(wanted.offset(), wanted.length())?;
        if bytes.is_empty() {
            initialization_fsm.finish()?;
        } else {
            initialization_fsm.handle_input(wanted.offset(), bytes)?;
        }
    };
    let mut source = Source::new(File::open(segment_path)?)?;
    let mut demux_fsm = MediaSegmentDemuxFsm::new(movie);

    let mut count: u64 = 0;
    while let Some(wanted) = demux_fsm.wanted_input() {
        let bytes = source.read_at(wanted.offset(), wanted.length())?;
        let handed = if bytes.is_empty() {
            demux_fsm.finish()
        } else {
            demux_fsm.handle_input(wanted.offset(), bytes)
        };
        while let Some(sample) = demux_fsm.poll_sample() {
            println!(
                "track={} time={} size={} sync={}",
                sample.track_id(),
                sample.decode_time(),
                sample.data().len(),
                !sample.sample_flags().sample_is_non_sync_sample(),
            );
            count = count.saturating_add(1);
        }
        handed?;
    }
    println!("samples={count}");

    Ok(())
}
