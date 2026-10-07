//! Reads the samples a non-fragmented MP4 file served over HTTP carries, one line per sample
//!
//! Each read the demux FSM wants is fetched with a range request, of the length it names or a
//! mebibyte, whichever is more, so a movie lying after its media data is read as
//! `demux_non_fragmented` reads it from a file. A server that does not answer a range request with
//! partial content of a stated length, no more than was asked for, is refused; a range it cannot
//! satisfy ends the file. An `https` server is checked against the Mozilla root certificates. The
//! output matches `demux_non_fragmented` for the same file.
//!
//! Usage: `cargo run -p isobmff-examples --example demux_non_fragmented_http -- <url>`

use core::error::Error;
use std::env;
use std::io::Read;

use isobmff::structure::MovieDemuxFsm;
use ureq::Agent;

fn main() -> Result<(), Box<dyn Error>> {
    let url = env::args()
        .nth(1)
        .ok_or("usage: demux_non_fragmented_http <url>")?;

    let agent = Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .new_agent();
    let mut buffer = Vec::new();
    let mut demux_fsm = MovieDemuxFsm::new();

    let mut count: u64 = 0;
    while let Some(wanted) = demux_fsm.wanted_input() {
        let length = wanted.length().unwrap_or(0).max(1024 * 1024);
        let last = wanted
            .offset()
            .checked_add(length.saturating_sub(1))
            .ok_or("the wanted range ends past u64::MAX")?;
        let mut response = agent
            .get(&url)
            .header("Range", format!("bytes={}-{last}", wanted.offset()))
            .call()?;
        let handed = match response.status().as_u16() {
            206 => {
                // Why not `take(length)` on the reader: where the body runs longer than asked for,
                // reading stops short of its end, and ureq then closes the connection instead of
                // keeping it for the next request.
                if response
                    .body()
                    .content_length()
                    .is_none_or(|received| received > length)
                {
                    return Err(
                        "the server answered a range request with no length or more than was asked for".into(),
                    );
                }
                buffer.clear();
                response.body_mut().as_reader().read_to_end(&mut buffer)?;
                demux_fsm.handle_input(wanted.offset(), &buffer)
            }
            416 => demux_fsm.finish(),
            status => {
                return Err(format!("the server answered a range request with {status}").into());
            }
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
