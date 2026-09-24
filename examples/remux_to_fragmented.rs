//! Rewrites a non-fragmented MP4 file as a fragmented one, carrying every track over
//!
//! Each track of the source movie keeps its boxes but for the sample tables, which are emptied to
//! its sample description, so its samples lie in the fragments alone; the movie header is made
//! anew from the source's timescale, and the boxes of the movie other than its tracks are dropped.
//! The samples are written in decode time across the tracks, whatever order the source lays them
//! down in, so each fragment carries the same stretch of time of every track. A fragment opens at
//! every sync sample of a track that carries a sync sample table, so a source with no such track
//! is written as one fragment.
//!
//! Usage: `cargo run -p isobmff-examples --example remux_to_fragmented -- <in.mp4> <out.mp4>`

use core::error::Error;
use std::env;
use std::fs::File;
use std::io::BufWriter;

use isobmff::boxes::{
    ChunkOffsetBox, ChunkOffsets, MovieBox, SampleSizeBox, SampleSizeEntries, SampleSizes,
    SampleTableBox, SampleToChunkBox, TimeToSampleBox,
};
use isobmff::io::blocking::{FragmentedMuxer, NonFragmentedDemuxer};
use isobmff::sample::Sample;

fn main() -> Result<(), Box<dyn Error>> {
    let usage = "usage: remux_to_fragmented <in.mp4> <out.mp4>";
    let mut arguments = env::args().skip(1);
    let input = arguments.next().ok_or(usage)?;
    let output = arguments.next().ok_or(usage)?;

    let mut demuxer = NonFragmentedDemuxer::new(File::open(input)?)?;
    let first = demuxer.next().transpose()?;
    let source = demuxer.movie().ok_or("the file carries no movie")?;

    let mut movie = MovieBox::new_fragmented(source.mvhd().timescale(), source.trak().to_vec())
        .ok_or("the movie declares no track")?;
    let mut cut_tracks = Vec::new();
    let mut queues = Vec::new();
    for track in source.trak() {
        let track_id = track.tkhd().track_id();
        let sample_table = track.mdia().minf().stbl();
        if sample_table.stss().is_some() {
            cut_tracks.push(track_id);
        }
        queues.push((track_id, track.mdia().mdhd().timescale(), Vec::new()));
        *movie
            .mdia_mut(track_id)
            .ok_or("the movie lost a track it declares")?
            .minf_mut()
            .stbl_mut() = SampleTableBox::new(
            sample_table.stsd().clone(),
            TimeToSampleBox::new(Vec::new()),
            SampleToChunkBox::new(Vec::new()),
            SampleSizes::Stsz(SampleSizeBox::new(SampleSizeEntries::PerSample(Vec::new()))),
            ChunkOffsets::Stco(ChunkOffsetBox::new(Vec::new())),
        );
    }

    let mut muxer = FragmentedMuxer::new(BufWriter::new(File::create(output)?));
    if let Some(file_type) = demuxer.file_type() {
        muxer.handle_file_type(file_type.clone())?;
    }
    muxer.handle_movie(movie)?;
    let mut sequence_number: u32 = 0;
    let mut write_fragment = |fragment: &mut Vec<Sample>| -> Result<(), Box<dyn Error>> {
        sequence_number = sequence_number
            .checked_add(1)
            .ok_or("more fragments than a sequence number counts")?;
        muxer.begin_fragment(sequence_number)?;
        fragment.sort_by_key(Sample::track_id);
        for sample in fragment.drain(..) {
            muxer.handle_sample(sample)?;
        }
        Ok(muxer.finish_fragment()?)
    };

    let mut samples = first.into_iter().map(Ok).chain(demuxer);
    let mut fragment = Vec::new();
    let mut carries_cut_track = false;
    loop {
        while queues.iter().any(|(_, _, queue)| queue.is_empty()) {
            let Some(sample) = samples.next() else {
                break;
            };
            let sample = sample?;
            queues
                .iter_mut()
                .find(|(track_id, _, _)| *track_id == sample.track_id())
                .ok_or("a sample of a track the movie does not declare")?
                .2
                .push(sample);
        }
        let Some((_, _, queue)) = queues
            .iter_mut()
            .filter(|(_, _, queue)| !queue.is_empty())
            .min_by(|(_, timescale, queue), (_, other_timescale, other)| {
                let time = queue.first().map_or(0, Sample::decode_time);
                let other_time = other.first().map_or(0, Sample::decode_time);
                u128::from(time)
                    .saturating_mul(u128::from(*other_timescale))
                    .cmp(&u128::from(other_time).saturating_mul(u128::from(*timescale)))
            })
        else {
            break;
        };
        let sample = queue.remove(0);
        let is_cut_track = cut_tracks.contains(&sample.track_id());
        if is_cut_track && carries_cut_track && !sample.sample_flags().sample_is_non_sync_sample() {
            write_fragment(&mut fragment)?;
            carries_cut_track = false;
        }
        carries_cut_track |= is_cut_track;
        fragment.push(sample);
    }
    if !fragment.is_empty() {
        write_fragment(&mut fragment)?;
    }
    muxer.finish()?;

    Ok(())
}
