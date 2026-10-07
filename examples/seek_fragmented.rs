//! Reads the samples of a fragmented MP4 file from the fragment its `mfra` names for a time on
//!
//! Each track's `tfra` gives the fragment holding its latest sync sample at or before the time —
//! in milliseconds of the track's media presentation time, edit lists not applied — or its
//! earliest one where the time comes before them all, and the reading resumes at the first of
//! those fragments in the file, from its first sample on: where every fragment opens at a sync
//! sample, every track starts at one. A file that does not close with an `mfra`, or whose `mfra`
//! lists no entry, is refused.
//!
//! Usage: `cargo run -p isobmff-examples --example seek_fragmented -- <in.mp4> <milliseconds>`

use core::error::Error;
use std::env;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use isobmff::sample::movie_fragment_random_access::sync_sample_at;
use isobmff::structure::MovieDemuxFsm;

fn main() -> Result<(), Box<dyn Error>> {
    let usage = "usage: seek_fragmented <in.mp4> <milliseconds>";
    let mut arguments = env::args().skip(1);
    let path = arguments.next().ok_or(usage)?;
    let milliseconds: u64 = arguments.next().ok_or(usage)?.parse()?;

    let mut file = File::open(path)?;
    let file_len = file.metadata()?.len();
    let mut buffer = vec![0; 1024 * 1024];
    let mut demux_fsm = MovieDemuxFsm::new();
    while demux_fsm.movie().is_none() {
        let wanted = demux_fsm
            .wanted_input()
            .ok_or("the file carries no movie")?;
        file.seek(SeekFrom::Start(wanted.offset()))?;
        let read = file.read(&mut buffer)?;
        if read == 0 {
            demux_fsm.finish()?;
        } else {
            demux_fsm.handle_input(wanted.offset(), buffer.get(..read).unwrap_or_default())?;
        }
    }
    demux_fsm.resume_at_movie_fragment_random_access(file_len)?;
    while let Some(wanted) = demux_fsm.wanted_input() {
        file.seek(SeekFrom::Start(wanted.offset()))?;
        let read = file.read(&mut buffer)?;
        if read == 0 {
            demux_fsm.finish()?;
        } else {
            demux_fsm.handle_input(wanted.offset(), buffer.get(..read).unwrap_or_default())?;
        }
    }
    let movie = demux_fsm.movie().ok_or("the file carries no movie")?;
    let random_access = demux_fsm
        .movie_fragment_random_access()
        .ok_or("the file does not close with an mfra")?;
    let mut moof_offsets = Vec::new();
    for tfra in random_access.tfra() {
        let timescale = movie
            .trak()
            .iter()
            .find(|track| track.tkhd().track_id() == tfra.track_id())
            .ok_or("an mfra names a track the movie does not declare")?
            .mdia()
            .mdhd()
            .timescale();
        let time = milliseconds
            .checked_mul(u64::from(timescale))
            .ok_or("the time runs past what 64 bits count")?
            / 1_000;
        let entry = sync_sample_at(tfra, time)
            .or_else(|| tfra.entries().iter().min_by_key(|entry| entry.time()));
        if let Some(entry) = entry {
            moof_offsets.push(entry.moof_offset());
        }
    }
    let earliest = moof_offsets
        .into_iter()
        .min()
        .ok_or("the mfra lists no entry")?;
    demux_fsm.resume_at(earliest)?;

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
