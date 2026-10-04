//! Rewrites a non-fragmented MP4 file as a fragmented one, carrying every track over
//!
//! Each track of the source movie keeps its boxes but for the sample tables, which are emptied to
//! its sample description, so its samples lie in the fragments alone; the movie header is made
//! anew from the source's timescale, and the boxes of the movie other than its tracks are dropped.
//! The samples are written in decode time across the tracks, whatever order the source lays them
//! down in, so each fragment carries the same stretch of time of every track. A fragment opens at
//! every sync sample of a track that carries a sync sample table, so a source with no such track
//! is written as one fragment. The source's `ftyp` is carried over, unless it lists a brand the
//! `default-base-is-moof` of the fragments shall not be used under — `isom` and the others earlier
//! than `iso5` — where the `iso6` one the muxer lays down takes its place.
//!
//! Usage: `cargo run -p isobmff-examples --example remux_to_fragmented -- <in.mp4> <out.mp4>`

use core::error::Error;
use std::env;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};

use isobmff::boxes::{
    ChunkOffsetBox, ChunkOffsets, MovieBox, SampleSizeBox, SampleSizeEntries, SampleSizes,
    SampleTableBox, SampleToChunkBox, TimeToSampleBox,
};
use isobmff::sample::Sample;
use isobmff::structure::{FragmentedMuxFsm, NonFragmentedDemuxFsm};

/// The samples of one track read ahead of the others, waiting their turn in decode time
struct TrackQueue {
    track_id: u32,
    timescale: u32,
    samples: Vec<Sample>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let usage = "usage: remux_to_fragmented <in.mp4> <out.mp4>";
    let mut arguments = env::args().skip(1);
    let input = arguments.next().ok_or(usage)?;
    let output = arguments.next().ok_or(usage)?;

    let mut input_file = File::open(input)?;
    let mut buffer = vec![0; 1024 * 1024];
    let mut demux_fsm = NonFragmentedDemuxFsm::new();
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
    let source_movie = demux_fsm.movie().ok_or("the file carries no movie")?;

    let mut movie = MovieBox::new_fragmented(
        source_movie.mvhd().timescale(),
        source_movie.trak().to_vec(),
    )
    .ok_or("the movie declares no track")?;
    let mut cut_tracks = Vec::new();
    let mut queues = Vec::new();
    for track in source_movie.trak() {
        let track_id = track.tkhd().track_id();
        let sample_table = track.mdia().minf().stbl();
        if sample_table.stss().is_some() {
            cut_tracks.push(track_id);
        }
        queues.push(TrackQueue {
            track_id,
            timescale: track.mdia().mdhd().timescale(),
            samples: Vec::new(),
        });
        *movie
            .trak_mut(track_id)
            .ok_or("the movie lost a track it declares")?
            .mdia_mut()
            .minf_mut()
            .stbl_mut() = SampleTableBox::new(
            sample_table.stsd().clone(),
            TimeToSampleBox::new(Vec::new()),
            SampleToChunkBox::new(Vec::new()),
            SampleSizes::Stsz(SampleSizeBox::new(SampleSizeEntries::PerSample(Vec::new()))),
            ChunkOffsets::Stco(ChunkOffsetBox::new(Vec::new())),
        );
    }

    let mut output_file = BufWriter::new(File::create(output)?);
    let mut mux_fsm = FragmentedMuxFsm::new();
    if let Some(file_type) = demux_fsm
        .file_type()
        .filter(|file_type| !file_type.forbids_default_base_is_moof())
    {
        mux_fsm.handle_file_type(file_type.clone())?;
    }
    mux_fsm.handle_movie(movie)?;
    let mut sequence_number: u32 = 0;
    let mut write_fragment = |fragment: &mut Vec<Sample>| -> Result<(), Box<dyn Error>> {
        sequence_number = sequence_number
            .checked_add(1)
            .ok_or("more fragments than a sequence number counts")?;
        mux_fsm.begin_fragment(sequence_number)?;
        fragment.sort_by_key(Sample::track_id);
        for sample in fragment.drain(..) {
            mux_fsm.handle_sample(sample)?;
        }
        mux_fsm.finish_fragment()?;
        while let Some(chunk) = mux_fsm.poll_output() {
            output_file.write_all(&chunk)?;
        }

        Ok(())
    };

    let mut fragment = Vec::new();
    let mut carries_cut_track = false;
    loop {
        loop {
            while let Some(sample) = demux_fsm.poll_sample() {
                queues
                    .iter_mut()
                    .find(|queue| queue.track_id == sample.track_id())
                    .ok_or("a sample of a track the movie does not declare")?
                    .samples
                    .push(sample);
            }
            handed?;
            if queues.iter().all(|queue| !queue.samples.is_empty()) {
                break;
            }
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
        let Some(queue) = queues
            .iter_mut()
            .filter(|queue| !queue.samples.is_empty())
            .min_by(|queue, other| {
                let time = queue.samples.first().map_or(0, Sample::decode_time);
                let other_time = other.samples.first().map_or(0, Sample::decode_time);
                u128::from(time)
                    .saturating_mul(u128::from(other.timescale))
                    .cmp(&u128::from(other_time).saturating_mul(u128::from(queue.timescale)))
            })
        else {
            break;
        };
        let sample = queue.samples.remove(0);
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
    mux_fsm.finish()?;
    while let Some(chunk) = mux_fsm.poll_output() {
        output_file.write_all(&chunk)?;
    }
    output_file.flush()?;

    Ok(())
}
