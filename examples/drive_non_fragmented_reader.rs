//! Reads the samples a non-fragmented MP4 file carries with the reader alone, one line per sample
//!
//! No demuxer of the `io` feature is involved: the file is handed to the reader a cut at a time,
//! and the bytes it names that the file has already passed — the media data of a movie lying after
//! it — are fetched through a second handle sought back to them.
//!
//! Usage: `cargo run -p isobmff-examples --example drive_non_fragmented_reader -- <in.mp4>`

use core::error::Error;
use std::env;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use isobmff::structure::NonFragmentedReader;

fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args()
        .nth(1)
        .ok_or("usage: drive_non_fragmented_reader <in.mp4>")?;
    let mut file = File::open(&path)?;
    let mut fetcher = File::open(path)?;

    let mut reader = NonFragmentedReader::new();
    let mut cut = vec![0; 64 * 1024];
    let mut count: u64 = 0;
    let mut finished = false;
    while !finished {
        let position = file.stream_position()?;
        match reader.wanted_extent() {
            Some(wanted) if wanted.start < position => {
                fetcher.seek(SeekFrom::Start(wanted.start))?;
                let read = fetcher.read(&mut cut)?;
                reader.handle_data(wanted.start, cut.get(..read).unwrap_or_default())?;
            }
            _ => {
                let read = file.read(&mut cut)?;
                if read == 0 {
                    reader.finish()?;
                    finished = true;
                } else {
                    reader.handle_input(cut.get(..read).unwrap_or_default())?;
                }
            }
        }
        while let Some(sample) = reader.poll_sample() {
            println!(
                "track={} time={} size={} sync={}",
                sample.track_id(),
                sample.decode_time(),
                sample.data().len(),
                !sample.sample_flags().sample_is_non_sync_sample(),
            );
            count = count.saturating_add(1);
        }
    }
    println!("samples={count}");

    Ok(())
}
