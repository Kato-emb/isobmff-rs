//! Reads the samples a non-fragmented MP4 file carries with the demux FSM alone, one line per sample
//!
//! No driver of the `io` feature is involved: the file is read where the demux FSM says — the
//! bytes it names as lacking that the file has already passed, the media data of a movie lying
//! after it, or else on from where its input stands — and each read is handed to it. The output
//! matches `demux_non_fragmented`.
//!
//! Usage: `cargo run -p isobmff-examples --example drive_non_fragmented_demux_fsm -- <in.mp4>`

use core::error::Error;
use std::env;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use isobmff::structure::NonFragmentedDemuxFsm;

fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args()
        .nth(1)
        .ok_or("usage: drive_non_fragmented_demux_fsm <in.mp4>")?;
    let mut file = File::open(path)?;

    let mut demux_fsm = NonFragmentedDemuxFsm::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut count: u64 = 0;
    while let Some(wanted) = demux_fsm.wanted_input() {
        file.seek(SeekFrom::Start(wanted.offset()))?;
        let read = file.read(&mut buffer)?;
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
