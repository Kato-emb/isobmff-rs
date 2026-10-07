//! Reads the samples a fragmented MP4 file carries over TCP, one line per sample
//!
//! The file is taken in the order it is sent, from its first byte, so a fragment whose media data
//! lies before it is refused: the connection cannot go back to it. The output matches
//! `demux_fragmented` for the same file.
//!
//! Usage: `cargo run -p isobmff-examples --example demux_fragmented_tcp -- <host:port>`

use core::error::Error;
use std::env;
use std::io::Read;
use std::net::TcpStream;

use isobmff::structure::MovieDemuxFsm;

fn main() -> Result<(), Box<dyn Error>> {
    let address = env::args()
        .nth(1)
        .ok_or("usage: demux_fragmented_tcp <host:port>")?;

    let mut stream = TcpStream::connect(address)?;
    let mut buffer = vec![0; 1024 * 1024];
    let mut demux_fsm = MovieDemuxFsm::new();

    let mut count: u64 = 0;
    while let Some(wanted) = demux_fsm.wanted_input() {
        if wanted.length().is_some() {
            return Err("a fragment names media data the connection has passed".into());
        }
        let read = stream.read(&mut buffer)?;
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
