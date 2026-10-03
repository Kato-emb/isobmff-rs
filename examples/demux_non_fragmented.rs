//! Reads the samples a non-fragmented MP4 file carries, one line per sample
//!
//! Usage: `cargo run -p isobmff-examples --example demux_non_fragmented -- <in.mp4>`

use core::error::Error;
use std::env;
use std::fs::File;

use isobmff::io::blocking::Source;
use isobmff::structure::NonFragmentedDemuxFsm;

fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args()
        .nth(1)
        .ok_or("usage: demux_non_fragmented <in.mp4>")?;

    let mut source = Source::new(File::open(path)?)?;
    let mut demux_fsm = NonFragmentedDemuxFsm::new();

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
