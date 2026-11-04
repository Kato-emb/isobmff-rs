//! Writes the samples of a movie file or a segment into another file structure: fragmented, not fragmented, or a segment
//!
//! ```text
//! cargo run --example remux -- movie.mp4 fragmented.mp4 fragmented
//! cargo run --example remux -- fragmented.mp4 movie.mp4 non-fragmented
//! cargo run --example remux -- fragmented.mp4 segment.mp4 segment
//! cargo run --example remux -- movie.mp4 fragmented.mp4 fragmented --random-access
//! cargo run --example remux -- fragmented.mp4 segment.mp4 segment --segment-index
//! cargo run --example remux -- fragmented.mp4 movie.mp4 non-fragmented --recoverable
//! ```
//!
//! `--random-access` ends a fragmented file with an `mfra` that lists the
//! sync samples of its fragments, and `--segment-index` starts a segment with
//! a `sidx` that indexes each `moof` and its `mdat` as a subsegment. The
//! muxer reserves the space of the `sidx` before the first `moof`, so it is
//! told the number of subsegments first: the source is read once for its
//! boxes alone to count its samples. As the muxer finishes, it writes the
//! `sidx` over the space reserved, an output that goes over what the
//! destination holds already. `--recoverable` writes a file that is not
//! fragmented as movie fragments, readable should the example stop short,
//! and lays it out again as it finishes: the `moov` at the end, then the
//! header of an `mdat` that holds the fragments over the `free` after the
//! `ftyp`, another output over what is written.
//!
//! A demuxer reads the source as the `demux` example does, seeking to the
//! bytes it wants. Its movie goes to a muxer as it is, and the samples the
//! demuxer returns go to the muxer a run at a time: each run is a `moof` and
//! an `mdat` of a fragmented file or a segment, or an `mdat` of a file that is
//! not fragmented, whose `moov` the muxer writes at the end, its sample tables
//! filled from the samples. Each box the muxer writes goes to the destination
//! as it comes out, where its offset says.
//!
//! The muxer takes the brands of a fragmented file or a segment where they
//! name one of `iso5` to `isod`: the source's brands, with `iso6` added where
//! they name none. A segment is written after the initialization segment of
//! its movie, which a second muxer writes, so the destination plays on its
//! own.

use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};

use isobmff::boxes::FileTypeBox;
use isobmff::{
    DemuxEvent, Demuxer, FileStructure, FourCc, FragmentedOptions, Muxer, NonFragmentedOptions,
    SegmentOptions, supports_default_base_is_moof,
};

/// Samples the muxer takes in one call: one `moof` and `mdat`, or one `mdat`
const SAMPLES_PER_RUN: usize = 64;

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let usage = "usage: remux <source> <destination> (fragmented [--random-access] | non-fragmented [--recoverable] | segment [--segment-index])";
    let (source, destination, structure, option) = (
        arguments.next().ok_or(usage)?,
        arguments.next().ok_or(usage)?,
        arguments.next().ok_or(usage)?,
        arguments.next(),
    );
    if arguments.next().is_some() {
        return Err(usage.into());
    }
    let mut muxer = match (structure.as_str(), option.as_deref()) {
        ("fragmented", option @ (None | Some("--random-access"))) => {
            let mut options = FragmentedOptions::default();
            options.movie_fragment_random_access = option.is_some();
            Muxer::fragmented(options)
        }
        ("non-fragmented", option @ (None | Some("--recoverable"))) => {
            let mut options = NonFragmentedOptions::default();
            options.recoverable = option.is_some();
            Muxer::non_fragmented(options)
        }
        ("segment", option @ (None | Some("--segment-index"))) => {
            let mut options = SegmentOptions::default();
            if option.is_some() {
                options.segment_index = Some(u32::try_from(
                    sample_count(&source)?.div_ceil(u64::try_from(SAMPLES_PER_RUN)?),
                )?);
            }
            Muxer::segment(options)
        }
        _ => return Err(usage.into()),
    };
    let file_structure = muxer.file_structure();
    let writes_fragments = file_structure != FileStructure::NonFragmented
        || option.as_deref() == Some("--recoverable");
    let mut source = BufReader::new(File::open(source)?);
    let mut destination = Destination {
        file: File::create(destination)?,
        written: 0,
        start: 0,
    };

    let mut demuxer = Demuxer::new();
    let mut pending = Vec::new();
    loop {
        let event = match demuxer.wanted_input() {
            Some(wanted) => {
                source.seek(SeekFrom::Start(wanted.offset()))?;
                match source.fill_buf()? {
                    [] => demuxer.finish(),
                    input => demuxer.handle_input(wanted.offset(), input),
                }
            }
            // The demuxer wants no input past a box that runs to the end of the file, which ends only once the file is finished.
            None => match demuxer.finish() {
                Ok(None) => break,
                finished => finished,
            },
        };
        pending.extend(std::iter::from_fn(|| demuxer.poll_sample()));
        if let Some(DemuxEvent::Movie { .. }) = event? {
            let movie = demuxer.movie().ok_or("moov without its value")?.clone();
            let file_type = brands_of(writes_fragments, demuxer.file_type());
            if file_structure == FileStructure::Segment {
                let mut initialization = Muxer::fragmented(FragmentedOptions::default());
                initialization.handle_file_type(file_type.clone())?;
                initialization.handle_movie(movie.clone())?;
                initialization.finish()?;
                destination.write_output(&mut initialization)?;
                destination.start = destination.written;
            }
            muxer.handle_file_type(file_type)?;
            muxer.handle_movie(movie)?;
            destination.write_output(&mut muxer)?;
        }
        while pending.len() >= SAMPLES_PER_RUN {
            muxer.handle_samples(pending.drain(..SAMPLES_PER_RUN))?;
            destination.write_output(&mut muxer)?;
        }
    }
    muxer.handle_samples(pending)?;
    muxer.finish()?;
    destination.write_output(&mut muxer)?;

    Ok(())
}

/// Returns the number of samples the source at `path` locates, reading its boxes but not its media data
///
/// A segment indexed by a `sidx` has as many subsegments as calls of the
/// muxer, which the muxer is told before the first, to reserve the space of
/// the `sidx`.
fn sample_count(path: &str) -> Result<u64, Box<dyn Error>> {
    let mut source = BufReader::new(File::open(path)?);
    let mut demuxer = Demuxer::new();
    let mut count = 0_u64;
    loop {
        let event = match demuxer.wanted_input() {
            Some(wanted) => {
                source.seek(SeekFrom::Start(wanted.offset()))?;
                match source.fill_buf()? {
                    [] => demuxer.finish(),
                    input => demuxer.handle_input(wanted.offset(), input),
                }
            }
            None => match demuxer.finish() {
                Ok(None) => break,
                finished => finished,
            },
        };
        match event? {
            Some(DemuxEvent::Movie { samples, .. }) => {
                let track_ids: Vec<u32> = demuxer
                    .movie()
                    .into_iter()
                    .flat_map(|movie| &movie.trak)
                    .map(|trak| trak.tkhd.track_id)
                    .collect();
                for track_id in track_ids {
                    demuxer.discard_track(track_id);
                }
                count = count.saturating_add(u64::try_from(samples.len())?);
            }
            Some(DemuxEvent::MovieFragment { samples, .. }) => {
                count = count.saturating_add(u64::try_from(samples.len())?);
            }
            _ => {}
        }
    }
    Ok(count)
}

/// Returns the brands of the destination, made of those of the source, `file_type`
///
/// A file the muxer writes movie fragments of, a recoverable one too, or a
/// segment takes brands that name one of `iso5` to `isod`: the source's, with
/// `iso6` added where they name none.
fn brands_of(writes_fragments: bool, file_type: Option<&FileTypeBox>) -> FileTypeBox {
    let mut file_type = file_type.cloned().unwrap_or_else(|| {
        let mut brands = FileTypeBox::default();
        brands.major_brand = FourCc::new(*b"mp41");
        brands
    });
    if writes_fragments && !supports_default_base_is_moof(&file_type) {
        file_type.compatible_brands.push(FourCc::new(*b"iso6"));
    }
    file_type
}

/// File the boxes are written to, where it ends, and where the file of the muxer written to it starts
struct Destination {
    file: File,
    written: u64,
    /// File offset in the destination of the first byte of the file the muxer writes: past the initialization segment of a segment
    start: u64,
}

impl Destination {
    /// Writes each box `muxer` has written where it goes, and prints where it lies
    ///
    /// An output at the end of the file extends it; one within what is
    /// written goes over it, the file seeking there and back; any other is
    /// refused.
    fn write_output(&mut self, muxer: &mut Muxer) -> Result<(), Box<dyn Error>> {
        while let Some(output) = muxer.poll_output() {
            let offset = self.start.saturating_add(output.offset());
            let bytes = output.bytes();
            let end = offset.saturating_add(u64::try_from(bytes.len())?);
            let box_type = bytes
                .get(4..8)
                .map(String::from_utf8_lossy)
                .unwrap_or_default();
            if offset == self.written {
                println!("[{box_type}] offset={offset} size={}", bytes.len());
                self.file.write_all(bytes)?;
                self.written = end;
            } else if end <= self.written {
                println!(
                    "[{box_type}] offset={offset} size={} (over what is written)",
                    bytes.len()
                );
                self.file.seek(SeekFrom::Start(offset))?;
                self.file.write_all(bytes)?;
                self.file.seek(SeekFrom::Start(self.written))?;
            } else {
                return Err(format!(
                    "output for file offset {offset}, neither the end {} of the file nor within it",
                    self.written
                )
                .into());
            }
        }
        Ok(())
    }
}
