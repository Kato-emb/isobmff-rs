//! Rewrites a fragmented MP4 file as a non-fragmented one, carrying every track over
//!
//! The movie header and the tracks of the source are kept, the modification time of the `mvhd` and
//! of each `tkhd` and `mdhd` set to now, its `mvex` and every other box of the movie dropped, and
//! the writer fills the sample tables of each track in from the samples and updates the durations
//! from them. A chunk opens wherever the samples pass to another track or another sample
//! description.
//!
//! Usage: `cargo run -p isobmff-examples --example remux_to_non_fragmented -- <in.mp4> <out.mp4>`

use core::error::Error;
use std::env;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use isobmff::boxes::MovieBox;
use isobmff::{MovieDemuxer, Mp4EpochSeconds, NonFragmentedMuxer};

fn main() -> Result<(), Box<dyn Error>> {
    let usage = "usage: remux_to_non_fragmented <in.mp4> <out.mp4>";
    let mut arguments = env::args().skip(1);
    let input = arguments.next().ok_or(usage)?;
    let output = arguments.next().ok_or(usage)?;

    let mut input_file = File::open(input)?;
    let mut buffer = vec![0; 1024 * 1024];
    let mut demuxer = MovieDemuxer::new();
    let mut handed = Ok(());
    while demuxer.movie().is_none() {
        handed?;
        let wanted = demuxer.wanted_input().ok_or("the file carries no movie")?;
        input_file.seek(SeekFrom::Start(wanted.offset()))?;
        let read = input_file.read(&mut buffer)?;
        handed = if read == 0 {
            demuxer.finish()
        } else {
            demuxer.handle_input(wanted.offset(), buffer.get(..read).unwrap_or_default())
        };
    }
    let movie = demuxer.movie().ok_or("the file carries no movie")?;
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
    let mut muxer = NonFragmentedMuxer::new();
    if let Some(file_type) = demuxer.file_type() {
        muxer.handle_file_type(file_type.clone())?;
    }
    muxer.handle_movie(movie)?;
    let mut chunk = None;
    loop {
        while let Some(sample) = demuxer.poll_output() {
            let described_by = Some((
                sample.properties().track_id,
                sample.properties().sample_description_index,
            ));
            if chunk != described_by {
                if chunk.is_some() {
                    muxer.finish_chunk()?;
                    while let Some(bytes) = muxer.poll_output() {
                        output_file.write_all(&bytes)?;
                    }
                }
                chunk = described_by;
                muxer.begin_chunk()?;
            }
            muxer.handle_sample(sample)?;
        }
        handed?;
        let Some(wanted) = demuxer.wanted_input() else {
            break;
        };
        input_file.seek(SeekFrom::Start(wanted.offset()))?;
        let read = input_file.read(&mut buffer)?;
        handed = if read == 0 {
            demuxer.finish()
        } else {
            demuxer.handle_input(wanted.offset(), buffer.get(..read).unwrap_or_default())
        };
    }
    if chunk.is_some() {
        muxer.finish_chunk()?;
    }
    muxer.finish()?;
    while let Some(bytes) = muxer.poll_output() {
        output_file.write_all(&bytes)?;
    }
    output_file.flush()?;

    Ok(())
}
