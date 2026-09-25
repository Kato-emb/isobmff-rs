//! Cuts a non-fragmented MP4 file into an initialization segment and media segments
//!
//! `init.mp4` carries an `ftyp` and the `moov` alone — the source's `ftyp`, or the `iso6` one the
//! muxer lays down where the source has none — the tracks of the source emptied of their samples
//! as `remux_to_fragmented` empties them. `0001.m4s`, `0002.m4s`, … each carry one fragment, after
//! a `styp` stating the brands of the source's `ftyp` where it has one. The samples are cut and
//! ordered as `remux_to_fragmented` cuts and orders them — in decode time across the tracks, a
//! segment opening at every sync sample of a track that carries a sync sample table — and each
//! keeps the decode time the source gives it, so the segments continue one another.
//!
//! Usage: `cargo run -p isobmff-examples --example segment_movie -- <in.mp4> <out_dir>`

use core::error::Error;
use std::env;
use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

use isobmff::boxes::{
    ChunkOffsetBox, ChunkOffsets, MovieBox, SampleSizeBox, SampleSizeEntries, SampleSizes,
    SampleTableBox, SampleToChunkBox, SegmentTypeBox, TimeToSampleBox,
};
use isobmff::io::blocking::{FragmentedMuxer, MediaSegmentMuxer, NonFragmentedDemuxer};
use isobmff::sample::Sample;

/// The samples of one track read ahead of the others, waiting their turn in decode time
struct TrackQueue {
    track_id: u32,
    timescale: u32,
    samples: Vec<Sample>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let usage = "usage: segment_movie <in.mp4> <out_dir>";
    let mut arguments = env::args().skip(1);
    let input = arguments.next().ok_or(usage)?;
    let output = PathBuf::from(arguments.next().ok_or(usage)?);

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

    let file_type = demuxer.file_type().cloned();
    let mut initialization =
        FragmentedMuxer::new(BufWriter::new(File::create(output.join("init.mp4"))?));
    if let Some(file_type) = &file_type {
        initialization.handle_file_type(file_type.clone())?;
    }
    initialization.handle_movie(movie)?;
    initialization.finish()?;

    let mut sequence_number: u32 = 0;
    let mut write_segment = |fragment: &mut Vec<Sample>| -> Result<(), Box<dyn Error>> {
        sequence_number = sequence_number
            .checked_add(1)
            .ok_or("more segments than a sequence number counts")?;
        let path = output.join(format!("{sequence_number:04}.m4s"));
        let mut segment = MediaSegmentMuxer::new(BufWriter::new(File::create(path)?));
        if let Some(file_type) = &file_type {
            segment.handle_segment_type(SegmentTypeBox::new(
                file_type.major_brand(),
                file_type.minor_version(),
                file_type.compatible_brands().to_vec(),
            ))?;
        }
        segment.begin_fragment(sequence_number)?;
        fragment.sort_by_key(Sample::track_id);
        for sample in fragment.drain(..) {
            segment.handle_sample(sample)?;
        }
        segment.finish_fragment()?;
        Ok(segment.finish()?)
    };

    let mut samples = first.into_iter().map(Ok).chain(demuxer);
    let mut fragment = Vec::new();
    let mut carries_cut_track = false;
    loop {
        while queues.iter().any(|queue| queue.samples.is_empty()) {
            let Some(sample) = samples.next() else {
                break;
            };
            let sample = sample?;
            queues
                .iter_mut()
                .find(|queue| queue.track_id == sample.track_id())
                .ok_or("a sample of a track the movie does not declare")?
                .samples
                .push(sample);
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
            write_segment(&mut fragment)?;
            carries_cut_track = false;
        }
        carries_cut_track |= is_cut_track;
        fragment.push(sample);
    }
    if !fragment.is_empty() {
        write_segment(&mut fragment)?;
    }

    Ok(())
}
