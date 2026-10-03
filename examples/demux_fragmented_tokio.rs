//! Reads the samples a fragmented MP4 file carries on tokio, one line per sample
//!
//! The output matches `demux_fragmented`: one line per sample, followed by the sample count.
//!
//! Usage: `cargo run -p isobmff-examples --example demux_fragmented_tokio -- <in.mp4>`

use core::error::Error;
use std::env;

use isobmff::io::Source;
use isobmff::structure::FragmentedDemuxFsm;
use tokio::fs::File;
use tokio_util::compat::TokioAsyncReadCompatExt;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args()
        .nth(1)
        .ok_or("usage: demux_fragmented_tokio <in.mp4>")?;

    let mut source = Source::new(File::open(path).await?.compat()).await?;
    let mut demux_fsm = FragmentedDemuxFsm::new();

    let mut count: u64 = 0;
    while let Some(wanted) = demux_fsm.wanted_input() {
        let bytes = source.read_at(wanted.offset(), wanted.length()).await?;
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
