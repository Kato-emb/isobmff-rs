//! Reads the samples a fragmented MP4 file carries on tokio, one line per sample
//!
//! The output matches `demux_fragmented`: one line per sample, followed by the sample count.
//!
//! Usage: `cargo run -p isobmff-examples --example demux_fragmented_tokio -- <in.mp4>`

use core::error::Error;
use std::env;
use std::io::SeekFrom;

use isobmff::structure::MovieDemuxFsm;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args()
        .nth(1)
        .ok_or("usage: demux_fragmented_tokio <in.mp4>")?;

    let mut file = File::open(path).await?;
    let mut buffer = vec![0; 1024 * 1024];
    let mut demux_fsm = MovieDemuxFsm::new();

    let mut count: u64 = 0;
    while let Some(wanted) = demux_fsm.wanted_input() {
        file.seek(SeekFrom::Start(wanted.offset())).await?;
        let read = file.read(&mut buffer).await?;
        let handed = if read == 0 {
            demux_fsm.finish()
        } else {
            demux_fsm.handle_input(wanted.offset(), buffer.get(..read).unwrap_or_default())
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
