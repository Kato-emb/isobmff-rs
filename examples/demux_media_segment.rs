//! Reads the samples a media segment carries, one line per sample
//!
//! The movie the segment continues is taken from its initialization segment.
//!
//! Usage: `cargo run -p isobmff-examples --example demux_media_segment -- <initialization.mp4> <segment.m4s>`

use core::error::Error;
use std::env;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use isobmff::structure::{MediaSegmentDemuxFsm, MovieDemuxFsm};

fn main() -> Result<(), Box<dyn Error>> {
    let usage = "usage: demux_media_segment <initialization.mp4> <segment.m4s>";
    let mut arguments = env::args().skip(1);
    let initialization_path = arguments.next().ok_or(usage)?;
    let segment_path = arguments.next().ok_or(usage)?;

    let mut initialization_file = File::open(initialization_path)?;
    let mut buffer = vec![0; 1024 * 1024];
    let mut initialization_fsm = MovieDemuxFsm::new();
    let movie = loop {
        if let Some(movie) = initialization_fsm.movie() {
            break movie.clone();
        }
        let wanted = initialization_fsm
            .wanted_input()
            .ok_or("the initialization segment carries no movie")?;
        initialization_file.seek(SeekFrom::Start(wanted.offset()))?;
        let read = initialization_file.read(&mut buffer)?;
        if read == 0 {
            initialization_fsm.finish()?;
        } else {
            initialization_fsm
                .handle_input(wanted.offset(), buffer.get(..read).unwrap_or_default())?;
        }
    };
    let mut segment_file = File::open(segment_path)?;
    let mut demux_fsm = MediaSegmentDemuxFsm::new(movie)?;

    let mut count: u64 = 0;
    while let Some(wanted) = demux_fsm.wanted_input() {
        segment_file.seek(SeekFrom::Start(wanted.offset()))?;
        let read = segment_file.read(&mut buffer)?;
        let handed = if read == 0 {
            demux_fsm.finish()
        } else {
            demux_fsm.handle_input(wanted.offset(), buffer.get(..read).unwrap_or_default())
        };
        while let Some(sample) = demux_fsm.poll_sample() {
            println!(
                "track={} time={} size={} sync={}",
                sample.properties().track_id,
                sample.properties().decode_time,
                sample.data().len(),
                !sample.properties().sample_flags.sample_is_non_sync_sample(),
            );
            count = count.saturating_add(1);
        }
        handed?;
    }
    println!("samples={count}");

    Ok(())
}
