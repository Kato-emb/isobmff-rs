//! Reads the samples a non-fragmented MP4 file carries, one line per sample
//!
//! Usage: `cargo run -p isobmff-examples --example demux_non_fragmented -- <in.mp4>`

use core::error::Error;
use std::env;
use std::fs::File;

use isobmff::io::blocking::NonFragmentedDemuxer;

fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args()
        .nth(1)
        .ok_or("usage: demux_non_fragmented <in.mp4>")?;

    let demuxer = NonFragmentedDemuxer::new(File::open(path)?)?;

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
