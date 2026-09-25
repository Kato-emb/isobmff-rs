//! Rewrites a fragmented MP4 file as a non-fragmented one, carrying every track over
//!
//! The movie header and the tracks of the source are kept, their modification time set to now, its
//! `mvex` and every other box of the movie dropped, and the writer fills the sample tables of each
//! track in from the samples. A chunk opens wherever the samples pass to another track or another
//! sample description.
//!
//! Usage: `cargo run -p isobmff-examples --example remux_to_non_fragmented -- <in.mp4> <out.mp4>`

use core::error::Error;
use std::env;
use std::fs::File;
use std::io::BufWriter;
use std::time::{SystemTime, UNIX_EPOCH};

use isobmff::boxes::MovieBox;
use isobmff::core::Mp4EpochSeconds;
use isobmff::io::blocking::{FragmentedDemuxer, NonFragmentedMuxer};

fn main() -> Result<(), Box<dyn Error>> {
    let usage = "usage: remux_to_non_fragmented <in.mp4> <out.mp4>";
    let mut arguments = env::args().skip(1);
    let input = arguments.next().ok_or(usage)?;
    let output = arguments.next().ok_or(usage)?;

    let mut demuxer = FragmentedDemuxer::new(File::open(input)?)?;
    let first = demuxer.next().transpose()?;
    let source = demuxer.movie().ok_or("the file carries no movie")?;
    let now =
        Mp4EpochSeconds::from_unix_seconds(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
            .ok_or("the clock is past what a header states")?;
    let tracks = source
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
        source.mvhd().clone().with_modification_time(now),
        tracks,
        None,
    )
    .ok_or("the movie declares no track")?;

    let mut muxer = NonFragmentedMuxer::new(BufWriter::new(File::create(output)?));
    if let Some(file_type) = demuxer.file_type() {
        muxer.handle_file_type(file_type.clone())?;
    }
    muxer.handle_movie(movie)?;
    let mut chunk = None;
    for sample in first.into_iter().map(Ok).chain(demuxer) {
        let sample = sample?;
        let described_by = Some((sample.track_id(), sample.sample_description_index()));
        if chunk != described_by {
            chunk = described_by;
            muxer.begin_chunk()?;
        }
        muxer.handle_sample(sample)?;
    }
    muxer.finish()?;

    Ok(())
}
