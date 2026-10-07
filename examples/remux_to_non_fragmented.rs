//! Rewrites a fragmented MP4 file as a non-fragmented one, carrying every track over
//!
//! The movie header and the tracks of the source are kept, the modification time of the `mvhd` and
//! of each `tkhd` and `mdhd` set to now, its `mvex` and every other box of the movie dropped, and
//! the writer fills the sample tables of each track in from the samples and states the durations
//! from them. A chunk opens wherever the samples pass to another track or another sample
//! description.
//!
//! Usage: `cargo run -p isobmff-examples --example remux_to_non_fragmented -- <in.mp4> <out.mp4>`

use core::error::Error;
use std::env;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use isobmff::Mp4EpochSeconds;
use isobmff::boxes::MovieBox;
use isobmff::structure::{MovieDemuxFsm, NonFragmentedMuxFsm};

fn main() -> Result<(), Box<dyn Error>> {
    let usage = "usage: remux_to_non_fragmented <in.mp4> <out.mp4>";
    let mut arguments = env::args().skip(1);
    let input = arguments.next().ok_or(usage)?;
    let output = arguments.next().ok_or(usage)?;

    let mut input_file = File::open(input)?;
    let mut buffer = vec![0; 1024 * 1024];
    let mut demux_fsm = MovieDemuxFsm::new();
    let mut handed = Ok(());
    while demux_fsm.movie().is_none() {
        handed?;
        let wanted = demux_fsm
            .wanted_input()
            .ok_or("the file carries no movie")?;
        input_file.seek(SeekFrom::Start(wanted.offset()))?;
        let read = input_file.read(&mut buffer)?;
        handed = if read == 0 {
            demux_fsm.finish()
        } else {
            demux_fsm.handle_input(wanted.offset(), buffer.get(..read).unwrap_or_default())
        };
    }
    let movie = demux_fsm.movie().ok_or("the file carries no movie")?;
    let now =
        Mp4EpochSeconds::from_unix_seconds(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
            .ok_or("the clock is past what a header states")?;
    let tracks = movie
        .trak()
        .iter()
        .cloned()
        .map(|mut track| {
            *track.tkhd_mut() = track.tkhd().clone().with_modification_time(now);
            *track.mdia_mut().mdhd_mut() = track.mdia().mdhd().clone().with_modification_time(now);
            track
        })
        .collect();
    let movie = MovieBox::new(
        movie.mvhd().clone().with_modification_time(now),
        tracks,
        None,
    )
    .ok_or("the movie declares no track")?;

    let mut output_file = BufWriter::new(File::create(output)?);
    let mut mux_fsm = NonFragmentedMuxFsm::new();
    if let Some(file_type) = demux_fsm.file_type() {
        mux_fsm.handle_file_type(file_type.clone())?;
    }
    mux_fsm.handle_movie(movie)?;
    let mut chunk = None;
    loop {
        while let Some(sample) = demux_fsm.poll_sample() {
            let described_by = Some((sample.track_id(), sample.sample_description_index()));
            if chunk != described_by {
                chunk = described_by;
                mux_fsm.begin_chunk()?;
                while let Some(chunk) = mux_fsm.poll_output() {
                    output_file.write_all(&chunk)?;
                }
            }
            mux_fsm.handle_sample(sample)?;
        }
        handed?;
        let Some(wanted) = demux_fsm.wanted_input() else {
            break;
        };
        input_file.seek(SeekFrom::Start(wanted.offset()))?;
        let read = input_file.read(&mut buffer)?;
        handed = if read == 0 {
            demux_fsm.finish()
        } else {
            demux_fsm.handle_input(wanted.offset(), buffer.get(..read).unwrap_or_default())
        };
    }
    mux_fsm.finish()?;
    while let Some(chunk) = mux_fsm.poll_output() {
        output_file.write_all(&chunk)?;
    }
    output_file.flush()?;

    Ok(())
}
