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

use isobmff::io::blocking::DemuxDriver;
use isobmff::sample::movie_fragment_random_access::sync_sample_at;
use isobmff::structure::FragmentedDemuxFsm;

fn main() -> Result<(), Box<dyn Error>> {
    let usage = "usage: seek_fragmented <in.mp4> <milliseconds>";
    let mut arguments = env::args().skip(1);
    let path = arguments.next().ok_or(usage)?;
    let milliseconds: u64 = arguments.next().ok_or(usage)?.parse()?;

    let mut driver = DemuxDriver::new(File::open(path)?, FragmentedDemuxFsm::new())?;
    driver.next().transpose()?;
    let mfra = driver
        .locate_movie_fragment_random_access()?
        .ok_or("the file does not close with an mfra")?;
    driver.fsm_mut().resume_at(mfra)?;
    driver.next().transpose()?;
    let movie = driver.fsm().movie().ok_or("the file carries no movie")?;
    let random_access = driver
        .fsm()
        .movie_fragment_random_access()
        .ok_or("the mfra did not read")?;
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
    driver.fsm_mut().resume_at(earliest)?;

    let mut count: u64 = 0;
    for sample in driver {
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
