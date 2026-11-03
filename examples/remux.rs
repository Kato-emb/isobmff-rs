//! Writes the samples of a movie file or a segment into another file structure: fragmented, not fragmented, or a segment
//!
//! ```text
//! cargo run --example remux -- movie.mp4 fragmented.mp4 fragmented
//! cargo run --example remux -- fragmented.mp4 movie.mp4 non-fragmented
//! cargo run --example remux -- fragmented.mp4 segment.mp4 segment
//! cargo run --example remux -- movie.mp4 fragmented.mp4 fragmented --random-access
//! cargo run --example remux -- fragmented.mp4 segment.mp4 segment --segment-index
//! ```
//!
//! `--random-access` ends a fragmented file with an `mfra` that lists the
//! sync samples of its fragments, and `--segment-index` starts a segment with
//! a `sidx` that indexes each `moof` and its `mdat` as a subsegment.
//!
//! A demuxer reads the source as the `demux` example does, seeking to the
//! bytes it wants. Its movie goes to a muxer as it is, and the samples the
//! demuxer returns go to the muxer a run at a time: each run is a `moof` and
//! an `mdat` of a fragmented file or a segment, or an `mdat` of a file that is
//! not fragmented, whose `moov` the muxer writes at the end, its sample tables
//! filled from the samples. Each box the muxer writes is appended to the
//! destination as it comes out.
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
use isobmff::{DemuxEvent, Demuxer, FileStructure, FourCc, Muxer};

/// Samples the muxer takes in one call: one `moof` and `mdat`, or one `mdat`
const SAMPLES_PER_RUN: usize = 64;

/// Brands under which a reader takes the `moof` as the base data offset of a track fragment
const FROM_ISO5: [[u8; 4]; 9] = [
    *b"iso5", *b"iso6", *b"iso7", *b"iso8", *b"iso9", *b"isoa", *b"isob", *b"isoc", *b"isod",
];

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let usage = "usage: remux <source> <destination> (fragmented [--random-access] | non-fragmented | segment [--segment-index])";
    let (source, destination, structure, option) = (
        arguments.next().ok_or(usage)?,
        arguments.next().ok_or(usage)?,
        arguments.next().ok_or(usage)?,
        arguments.next(),
    );
    if arguments.next().is_some() {
        return Err(usage.into());
    }
    let file_structure = match (structure.as_str(), option.as_deref()) {
        ("fragmented", option @ (None | Some("--random-access"))) => FileStructure::Fragmented {
            first_sequence_number: 1,
            random_access: option.is_some(),
        },
        ("non-fragmented", None) => FileStructure::NonFragmented,
        ("segment", option @ (None | Some("--segment-index"))) => FileStructure::Segment {
            first_sequence_number: 1,
            segment_index: option.is_some(),
        },
        _ => return Err(usage.into()),
    };
    let mut source = BufReader::new(File::open(source)?);
    let mut destination = Destination {
        file: File::create(destination)?,
        written: 0,
    };

    let mut demuxer = Demuxer::new();
    let mut muxer = Muxer::new(file_structure);
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
            let file_type = brands_of(file_structure, demuxer.file_type());
            if let FileStructure::Segment { .. } = file_structure {
                let mut initialization = Muxer::new(FileStructure::Fragmented {
                    first_sequence_number: 1,
                    random_access: false,
                });
                initialization.handle_file_type(file_type.clone())?;
                initialization.handle_movie(movie.clone())?;
                initialization.finish()?;
                destination.write_output(&mut initialization)?;
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

/// Returns the brands of the destination, made of those of the source, `file_type`
///
/// A fragmented file or a segment takes brands that name one of `iso5` to
/// `isod`: the source's, with `iso6` added where they name none.
fn brands_of(file_structure: FileStructure, file_type: Option<&FileTypeBox>) -> FileTypeBox {
    let mut file_type = file_type.cloned().unwrap_or_else(|| {
        let mut brands = FileTypeBox::default();
        brands.major_brand = FourCc::new(*b"mp41");
        brands
    });
    if file_structure != FileStructure::NonFragmented
        && !std::iter::once(&file_type.major_brand)
            .chain(&file_type.compatible_brands)
            .any(|brand| FROM_ISO5.contains(brand.as_bytes()))
    {
        file_type.compatible_brands.push(FourCc::new(*b"iso6"));
    }
    file_type
}

/// File the boxes are written to, and how many bytes are written
struct Destination {
    file: File,
    written: u64,
}

impl Destination {
    /// Appends each box `muxer` has written, and prints where it lies
    fn write_output(&mut self, muxer: &mut Muxer) -> Result<(), Box<dyn Error>> {
        while let Some(output) = muxer.poll_output() {
            let box_type = output
                .get(4..8)
                .map(String::from_utf8_lossy)
                .unwrap_or_default();
            println!("[{box_type}] offset={} size={}", self.written, output.len());
            self.file.write_all(&output)?;
            self.written = self.written.saturating_add(u64::try_from(output.len())?);
        }
        Ok(())
    }
}
