//! [`remux_to_fragmented`] and [`remux_to_non_fragmented`], a movie file written out again in the layout asked for

use std::io::{self, Read, Seek, SeekFrom, Write};

use isobmff::Mp4EpochSeconds;
use isobmff::boxes::{
    ChunkOffsetBox, ChunkOffsets, MovieBox, SampleSizeBox, SampleSizeEntries, SampleSizes,
    SampleTableBox, SampleToChunkBox, TimeToSampleBox,
};
use isobmff::sample::Sample;
use isobmff::sequence::OutputBytes;
use isobmff::structure::{self, FragmentedMuxFsm, MovieDemuxFsm, NonFragmentedMuxFsm};
use wasm_bindgen::prelude::wasm_bindgen;

use crate::Error;
use crate::demux::no_movie;

/// The most bytes read from the source at a time
const CUT_LENGTH: usize = 1024 * 1024;

/// The layout a file is written out in
#[wasm_bindgen]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Output {
    /// A movie whose samples all lie in movie fragments
    Fragmented,
    /// A movie whose sample tables declare every sample
    NonFragmented,
}

/// The samples of one track read ahead of the others, waiting their turn in decode time
struct TrackQueue {
    track_id: u32,
    timescale: u32,
    samples: Vec<Sample>,
}

/// Writes the movie file `source` carries, fragmented or not, to `sink` as a fragmented one
///
/// Each track keeps its boxes but for the sample tables, which are emptied to
/// its sample description, so its samples lie in the fragments alone; the
/// movie header is made anew from the source's timescale, and the boxes of the
/// movie other than its tracks are dropped. The samples are taken in decode
/// time across the tracks, so each fragment carries the same stretch of time
/// of every track, and are written track by track within it. A fragment opens
/// at every sync sample of a track whose `stbl` carries an `stss`, so a source
/// with no such track, a fragmented one among them, is written as one
/// fragment. The source's `ftyp` is carried over, unless it lists a brand the
/// `default-base-is-moof` of the fragments shall not be used under, where the
/// one the muxer lays down takes its place.
///
/// # Errors
///
/// The failure of `source` or `sink`; the reason the library refuses what it
/// reads or what it is to write, a file carrying no `moov` or none ahead of
/// its first `moof` among them; a movie declaring no track or two tracks of
/// the same `track_ID`, or a sample of a track it does not declare, as
/// [`InvalidData`](io::ErrorKind::InvalidData); or more fragments than a
/// sequence number counts.
pub(crate) fn remux_to_fragmented<S: Read + Seek, W: Write>(
    mut source: S,
    mut sink: W,
) -> Result<(), Error> {
    let mut cut = vec![0; CUT_LENGTH];
    let (mut demux_fsm, mut handed) = read_movie(&mut source, &mut cut)?;
    let source_movie = demux_fsm.movie().ok_or_else(no_movie)?;

    let mut movie = MovieBox::new_fragmented(
        source_movie.mvhd().timescale(),
        source_movie.trak().to_vec(),
    )
    .ok_or_else(unwritable_movie)?;
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
            .trak_by_id_mut(track_id)
            .ok_or_else(unwritable_movie)?
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

    let mut mux_fsm = FragmentedMuxFsm::new();
    if let Some(file_type) = demux_fsm
        .file_type()
        .filter(|file_type| !file_type.forbids_default_base_is_moof())
    {
        mux_fsm.handle_file_type(file_type.clone())?;
    }
    mux_fsm.handle_movie(movie)?;
    let mut sequence_number: u32 = 0;
    let mut write_fragment = |fragment: &mut Vec<Sample>| -> Result<(), Error> {
        sequence_number = sequence_number.checked_add(1).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "more fragments than a sequence number counts",
            )
        })?;
        mux_fsm.begin_fragment(sequence_number)?;
        fragment.sort_by_key(|sample| sample.properties().track_id);
        for sample in fragment.drain(..) {
            mux_fsm.handle_sample(sample)?;
        }
        mux_fsm.finish_fragment()?;
        write_output(&mut sink, || mux_fsm.poll_output())?;
        Ok(())
    };

    let mut fragment = Vec::new();
    let mut carries_cut_track = false;
    loop {
        loop {
            while let Some(sample) = demux_fsm.poll_sample() {
                queues
                    .iter_mut()
                    .find(|queue| queue.track_id == sample.properties().track_id)
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "a sample of a track the movie does not declare",
                        )
                    })?
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
            handed = handle_wanted(&mut demux_fsm, wanted.offset(), &mut source, &mut cut)?;
        }
        let Some(queue) = queues
            .iter_mut()
            .filter(|queue| !queue.samples.is_empty())
            .min_by(|queue, other| {
                let time = queue
                    .samples
                    .first()
                    .map_or(0, |sample| sample.properties().decode_time);
                let other_time = other
                    .samples
                    .first()
                    .map_or(0, |sample| sample.properties().decode_time);
                u128::from(time)
                    .saturating_mul(u128::from(other.timescale))
                    .cmp(&u128::from(other_time).saturating_mul(u128::from(queue.timescale)))
            })
        else {
            break;
        };
        let sample = queue.samples.remove(0);
        let is_cut_track = cut_tracks.contains(&sample.properties().track_id);
        if is_cut_track
            && carries_cut_track
            && !sample.properties().sample_flags.sample_is_non_sync_sample()
        {
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
    write_output(&mut sink, || mux_fsm.poll_output())?;
    sink.flush()?;

    Ok(())
}

/// Writes the movie file `source` carries, fragmented or not, to `sink` as a non-fragmented one
///
/// The movie header and the tracks of the source are kept, the modification
/// time of the `mvhd` and of each `tkhd` and `mdhd` set to `now`, its `mvex`
/// and every other box of the movie dropped, and the sample tables of each
/// track are filled in from the samples and the durations updated from them.
/// A chunk opens wherever the samples pass to another track or another sample
/// description. The source's `ftyp` is carried over.
///
/// # Errors
///
/// The failure of `source` or `sink`; the reason the library refuses what it
/// reads or what it is to write, a file carrying no `moov` or none ahead of
/// its first `moof` among them; or a movie declaring no track or two tracks of
/// the same `track_ID`, as [`InvalidData`](io::ErrorKind::InvalidData).
pub(crate) fn remux_to_non_fragmented<S: Read + Seek, W: Write>(
    mut source: S,
    mut sink: W,
    now: Mp4EpochSeconds,
) -> Result<(), Error> {
    let mut cut = vec![0; CUT_LENGTH];
    let (mut demux_fsm, mut handed) = read_movie(&mut source, &mut cut)?;
    let movie = demux_fsm.movie().ok_or_else(no_movie)?;
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
    .ok_or_else(unwritable_movie)?;

    let mut mux_fsm = NonFragmentedMuxFsm::new();
    if let Some(file_type) = demux_fsm.file_type() {
        mux_fsm.handle_file_type(file_type.clone())?;
    }
    mux_fsm.handle_movie(movie)?;
    let mut chunk = None;
    loop {
        while let Some(sample) = demux_fsm.poll_sample() {
            let described_by = Some((
                sample.properties().track_id,
                sample.properties().sample_description_index,
            ));
            if chunk != described_by {
                if chunk.is_some() {
                    mux_fsm.finish_chunk()?;
                    write_output(&mut sink, || mux_fsm.poll_output())?;
                }
                chunk = described_by;
                mux_fsm.begin_chunk()?;
            }
            mux_fsm.handle_sample(sample)?;
        }
        handed?;
        let Some(wanted) = demux_fsm.wanted_input() else {
            break;
        };
        handed = handle_wanted(&mut demux_fsm, wanted.offset(), &mut source, &mut cut)?;
    }
    if chunk.is_some() {
        mux_fsm.finish_chunk()?;
    }
    mux_fsm.finish()?;
    write_output(&mut sink, || mux_fsm.poll_output())?;
    sink.flush()?;

    Ok(())
}

/// Reads `source` until its movie has arrived, returning the demux FSM and how it took the last cut handed over
///
/// # Errors
///
/// The failure of `source`, or the reason the demux FSM refuses what it was
/// handed before the cut the movie completes in, a file carrying no `moov`
/// among them.
fn read_movie<S: Read + Seek>(
    source: &mut S,
    cut: &mut [u8],
) -> Result<(MovieDemuxFsm, Result<(), structure::Error>), Error> {
    let mut demux_fsm = MovieDemuxFsm::new();
    let mut handed = Ok(());
    while demux_fsm.movie().is_none() {
        handed?;
        let offset = demux_fsm.wanted_input().ok_or_else(no_movie)?.offset();
        handed = handle_wanted(&mut demux_fsm, offset, source, cut)?;
    }
    Ok((demux_fsm, handed))
}

/// Reads the cut at `offset` of `source` and hands it to `demux_fsm`, or declares the file over where nothing is left there
///
/// The inner result is how the demux FSM took the cut, a refusal to be
/// returned once the samples it completed before it are taken.
///
/// # Errors
///
/// The failure of `source`.
fn handle_wanted<S: Read + Seek>(
    demux_fsm: &mut MovieDemuxFsm,
    offset: u64,
    source: &mut S,
    cut: &mut [u8],
) -> io::Result<Result<(), structure::Error>> {
    source.seek(SeekFrom::Start(offset))?;
    let read = source.read(cut)?;
    Ok(if read == 0 {
        demux_fsm.finish()
    } else {
        demux_fsm.handle_input(offset, cut.get(..read).unwrap_or_default())
    })
}

/// Writes every output `poll_output` yields to `sink`, until it yields none
///
/// # Errors
///
/// The failure of `sink`.
fn write_output<W: Write>(
    sink: &mut W,
    mut poll_output: impl FnMut() -> Option<OutputBytes>,
) -> io::Result<()> {
    while let Some(bytes) = poll_output() {
        sink.write_all(&bytes)?;
    }
    Ok(())
}

/// Returns the failure of a movie the source declares that cannot be written as it is
fn unwritable_movie() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "the movie declares no track, or two tracks of the same track_ID",
    )
}

#[cfg(test)]
mod tests {
    use std::io;

    use isobmff::Mp4EpochSeconds;
    use isobmff::sample::Sample;
    use isobmff::structure::MovieDemuxFsm;
    use isobmff_test_support::{
        SAMPLE_CHUNKS, fragmented_file_with_movie_samples, non_fragmented_file,
        non_fragmented_file_samples,
    };

    use super::{CUT_LENGTH, handle_wanted, remux_to_fragmented, remux_to_non_fragmented};

    fn samples_of(file: Vec<u8>) -> Vec<Sample> {
        let mut source = io::Cursor::new(file);
        let mut cut = vec![0; CUT_LENGTH];
        let mut demux_fsm = MovieDemuxFsm::new();
        let mut samples = Vec::new();
        while let Some(wanted) = demux_fsm.wanted_input() {
            handle_wanted(&mut demux_fsm, wanted.offset(), &mut source, &mut cut)
                .unwrap()
                .unwrap();
            while let Some(sample) = demux_fsm.poll_sample() {
                samples.push(sample);
            }
        }
        samples
    }

    #[test]
    fn a_fragmented_file_written_non_fragmented_carries_the_same_samples() {
        let file = fragmented_file_with_movie_samples(true, false);
        let mut written = Vec::new();

        remux_to_non_fragmented(
            io::Cursor::new(file.bytes),
            &mut written,
            Mp4EpochSeconds::from_seconds(3_000_000_000),
        )
        .unwrap();

        assert_eq!(
            samples_of(written),
            [file.movie_samples, file.fragment_samples].concat()
        );
    }

    #[test]
    fn a_non_fragmented_file_written_fragmented_carries_the_same_samples() {
        let mut written = Vec::new();

        remux_to_fragmented(
            io::Cursor::new(non_fragmented_file(&SAMPLE_CHUNKS, true)),
            &mut written,
        )
        .unwrap();

        assert_eq!(samples_of(written), non_fragmented_file_samples());
    }
}
