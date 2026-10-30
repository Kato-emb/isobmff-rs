//! Writes the samples of a movie file or a segment into another file structure: fragmented, not fragmented, or a segment
//!
//! ```text
//! cargo run --example remux -- movie.mp4 fragmented.mp4 fragmented
//! cargo run --example remux -- fragmented.mp4 movie.mp4 non-fragmented
//! cargo run --example remux -- fragmented.mp4 segment.mp4 segment
//! ```
//!
//! A demuxer reads the source as the `demux` example does, seeking to the
//! bytes it wants. Its movie, its sample tables emptied, goes to a muxer, and
//! the samples the demuxer returns go to the muxer a run at a time: each run
//! is a `moof` and an `mdat` of a fragmented file or a segment, or an `mdat`
//! of a file that is not fragmented, whose `moov` the muxer writes at the end.
//! Each box the muxer writes is appended to the destination as it comes out.
//!
//! The muxer takes the movie of a fragmented file or a segment with a `trex`
//! for each track, and brands that name one of `iso5` to `isod`: the source's
//! brands, with `iso6` added where they name none. A segment is written after
//! the initialization segment of its movie, which a second muxer writes, so
//! the destination plays on its own.

use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

use isobmff::boxes::{
    ChunkOffsetVariant, FileTypeBox, MovieBox, MovieExtendsBox, SampleSizeVariant, TrackExtendsBox,
};
use isobmff::{DemuxEvent, Demuxer, FileStructure, FourCc, Muxer};

/// Samples the muxer takes in one call: one `moof` and `mdat`, or one `mdat`
const SAMPLES_PER_RUN: usize = 64;

/// Brands under which a reader takes the `moof` as the base data offset of a track fragment
const FROM_ISO5: [[u8; 4]; 9] = [
    *b"iso5", *b"iso6", *b"iso7", *b"iso8", *b"iso9", *b"isoa", *b"isob", *b"isoc", *b"isod",
];

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let usage = "usage: remux <source> <destination> fragmented|non-fragmented|segment";
    let (source, destination, structure) = (
        arguments.next().ok_or(usage)?,
        arguments.next().ok_or(usage)?,
        arguments.next().ok_or(usage)?,
    );
    let file_structure = match structure.as_str() {
        "fragmented" => FileStructure::Fragmented {
            first_sequence_number: 1,
        },
        "non-fragmented" => FileStructure::NonFragmented,
        "segment" => FileStructure::Segment {
            first_sequence_number: 1,
        },
        _ => return Err(usage.into()),
    };
    let mut source = File::open(source)?;
    let mut destination = Destination {
        file: File::create(destination)?,
        written: 0,
    };

    let mut demuxer = Demuxer::new();
    let mut muxer = Muxer::new(file_structure);
    let mut pending = Vec::new();
    let mut buffer = vec![0; 64 * 1024];
    let (mut buffered_at, mut buffered) = (0_u64, 0_usize);
    while let Some(wanted) = demuxer.wanted_input() {
        let buffered_end = buffered_at.saturating_add(u64::try_from(buffered)?);
        if !(buffered_at..buffered_end).contains(&wanted.offset()) {
            source.seek(SeekFrom::Start(wanted.offset()))?;
            buffered = source.read(&mut buffer)?;
            buffered_at = wanted.offset();
        }
        let event = match buffer.get(..buffered).ok_or("read past the buffer")? {
            [] => demuxer.finish(),
            input => demuxer.handle_input(buffered_at, input),
        };
        pending.extend(std::iter::from_fn(|| demuxer.poll_sample()));
        if let Some(DemuxEvent::Movie { .. }) = event? {
            let movie = demuxer.movie().ok_or("moov without its value")?.clone();
            let (file_type, movie) = destination_of(file_structure, demuxer.file_type(), movie);
            if let FileStructure::Segment { .. } = file_structure {
                let mut initialization = Muxer::new(FileStructure::Fragmented {
                    first_sequence_number: 1,
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

/// Returns the brands and the movie of the destination, made of those of the source, `file_type` and `movie`
///
/// The sample tables of the movie are emptied, as the samples the muxer
/// writes declare them, and the tables of what else the source states of
/// each sample, which the muxer does not write, are dropped. A fragmented file or a segment takes a `trex` for
/// each track, and brands that name one of `iso5` to `isod`.
fn destination_of(
    file_structure: FileStructure,
    file_type: Option<&FileTypeBox>,
    mut movie: MovieBox,
) -> (FileTypeBox, MovieBox) {
    for trak in &mut movie.trak {
        let stbl = &mut trak.mdia.minf.stbl;
        stbl.stts = Some(Default::default());
        stbl.ctts = None;
        stbl.stsc = Some(Default::default());
        stbl.sample_size = Some(SampleSizeVariant::SampleSize(Default::default()));
        stbl.chunk_offset = Some(ChunkOffsetVariant::ChunkOffset(Default::default()));
        stbl.stss = None;
        stbl.sdtp = None;
        stbl.stsh = None;
        stbl.padb = None;
        stbl.stdp = None;
        stbl.sbgp.clear();
        stbl.csgp.clear();
        stbl.subs.clear();
        stbl.ssrt.clear();
        stbl.saiz.clear();
        stbl.saio.clear();
    }
    let mut file_type = file_type.cloned().unwrap_or_else(|| {
        let mut brands = FileTypeBox::default();
        brands.major_brand = FourCc::new(*b"mp41");
        brands
    });
    if file_structure == FileStructure::NonFragmented {
        movie.mvex = None;
        return (file_type, movie);
    }

    let mut mvex = MovieExtendsBox::default();
    mvex.trex = movie
        .trak
        .iter()
        .map(|trak| {
            let mut trex = TrackExtendsBox::default();
            trex.track_id = trak.tkhd.track_id;
            trex.default_sample_description_index = 1;
            trex
        })
        .collect();
    movie.mvex = Some(mvex);
    if !std::iter::once(&file_type.major_brand)
        .chain(&file_type.compatible_brands)
        .any(|brand| FROM_ISO5.contains(brand.as_bytes()))
    {
        file_type.compatible_brands.push(FourCc::new(*b"iso6"));
    }
    (file_type, movie)
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
