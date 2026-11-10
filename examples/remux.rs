//! A file read with the demuxer and written again with a muxer, as a movie file or as a fragmented movie file
//!
//! The example reads every sample of a file with [`Demuxer`], then hands
//! them to [`MovieFileMuxer`], or with `--fragmented` to
//! [`FragmentedMovieFileMuxer`], which ends a fragment where one of the file
//! did.
//!
//! `cargo run --example remux -- <input> <output> [--fragmented]`

use isobmff::boxes::{FileTypeBox, MovieBox};
use isobmff::demuxer::{Error as DemuxError, Event};
use isobmff::muxer::{Error, MovieDescription, TrackDescription};
use isobmff::{
    Demuxer, DemuxerConfig, FragmentedMovieFileMuxer, MovieFileMuxer, MuxerConfig, SampleProperties,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let usage = "usage: remux <input> <output> [--fragmented]";
    let (input, output) = (
        arguments.next().ok_or(usage)?,
        arguments.next().ok_or(usage)?,
    );
    let fragmented = arguments.next().as_deref() == Some("--fragmented");
    let source = read(&std::fs::read(input)?).ok_or("no ftyp or no moov")?;
    let written = if fragmented {
        fragmented_movie_file(&source)?
    } else {
        movie_file(&source)?
    };
    std::fs::write(output, written)?;
    Ok(())
}

/// What the demuxer reads of a file: its `ftyp`, its `moov`, and every sample of its tracks with its bytes, in file order
#[derive(Debug, Clone, PartialEq)]
struct Read {
    /// The `ftyp` of the file
    file_type: FileTypeBox,
    /// The `moov` of the file
    movie: MovieBox,
    /// What the structure-data declares of each sample, with its bytes
    samples: Vec<(SampleProperties, Vec<u8>)>,
    /// Index in `samples` of the first sample read after each `moof`, in the
    /// order the boxes come
    fragment_starts: Vec<usize>,
    /// What the demuxer could not read
    errors: Vec<DemuxError>,
}

/// Reads `file` where the demuxer asks, subscribing to every track, or returns `None` where it holds no `ftyp` or no `moov`
#[must_use]
fn read(file: &[u8]) -> Option<Read> {
    let mut demuxer = Demuxer::new(DemuxerConfig::default());
    let (mut file_type, mut movie) = (None, None);
    let (mut samples, mut fragment_starts, mut errors) = (Vec::new(), Vec::new(), Vec::new());
    loop {
        while let Some(event) = demuxer.poll_event() {
            match event {
                Event::FileType(ftyp) => file_type = Some(ftyp),
                Event::Movie(moov) => {
                    for trak in &moov.trak {
                        demuxer.subscribe_track(trak.tkhd.track_id);
                    }
                    movie = Some(*moov);
                }
                Event::MovieFragment(_) => fragment_starts.push(samples.len()),
                Event::Sample(sample) => samples.push((sample.properties, sample.bytes.to_vec())),
                Event::Error(error) => errors.push(error),
                Event::SegmentType(_) | Event::SampleLocation(_) | Event::Box { .. } | _ => {}
            }
        }
        let Some(required) = demuxer.required_input() else {
            break;
        };
        match usize::try_from(required.position)
            .ok()
            .and_then(|position| file.get(position..))
        {
            Some(bytes) if !bytes.is_empty() => demuxer.handle_input(required.position, bytes),
            _ => demuxer.finish(),
        }
    }
    Some(Read {
        file_type: file_type?,
        movie: movie?,
        samples,
        fragment_starts,
        errors,
    })
}

/// Returns the movie file of the samples `source` holds: its `ftyp`, one `mdat` and the `moov` that declares them
///
/// # Errors
/// What [`MovieFileMuxer`] refuses of the movie or its samples.
fn movie_file(source: &Read) -> Result<Vec<u8>, Error> {
    let mut muxer = MovieFileMuxer::new(
        MovieDescription::from(&source.movie),
        MuxerConfig::default(),
        source.file_type.clone(),
    )?;
    for trak in &source.movie.trak {
        muxer.add_track(TrackDescription::from(trak))?;
    }
    let mut written = Vec::new();
    for (properties, bytes) in &source.samples {
        muxer.handle_sample(*properties, bytes)?;
        muxer.poll_output(&mut written);
    }
    let header = muxer.finish()?;
    muxer.poll_output(&mut written);
    if let Some(first) = usize::try_from(header.offset)
        .ok()
        .and_then(|offset| written.get_mut(offset..))
        .and_then(|rest| rest.get_mut(..header.bytes.len()))
    {
        first.copy_from_slice(&header.bytes);
    }
    Ok(written)
}

/// Returns the fragmented movie file of the samples `source` holds, a fragment ending where one of `source` does, or where the next sample would pass the limits of the muxer
///
/// # Errors
/// What [`FragmentedMovieFileMuxer`] refuses of the movie or its samples.
fn fragmented_movie_file(source: &Read) -> Result<Vec<u8>, Error> {
    let mut muxer = FragmentedMovieFileMuxer::new(
        MovieDescription::from(&source.movie),
        MuxerConfig::default(),
        source.file_type.clone(),
    )?;
    for trak in &source.movie.trak {
        muxer.add_track(TrackDescription::from(trak))?;
    }
    let mut written = Vec::new();
    for (index, (properties, bytes)) in source.samples.iter().enumerate() {
        if source.fragment_starts.binary_search(&index).is_ok() {
            muxer.finish_fragment()?;
        }
        match muxer.handle_sample(*properties, bytes) {
            Err(
                Error::BufferFull { .. }
                | Error::StructureData(isobmff::structure_data::Error::SampleCountOverLimit {
                    ..
                }),
            ) => {
                muxer.poll_output(&mut written);
                muxer.finish_fragment()?;
                muxer.poll_output(&mut written);
                muxer.handle_sample(*properties, bytes)?;
            }
            result => result?,
        }
        muxer.poll_output(&mut written);
    }
    muxer.finish()?;
    muxer.poll_output(&mut written);
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::{fragmented_movie_file, movie_file, read};

    #[test]
    fn a_file_written_again_as_a_movie_file_or_in_fragments_holds_the_same_samples() {
        for file in [
            include_bytes!("../crates/isobmff/tests/fixtures/movie_moov_last.mp4").as_slice(),
            include_bytes!("../crates/isobmff/tests/fixtures/fragmented.mp4"),
        ] {
            let source = read(file).unwrap();
            for written in [movie_file(&source), fragmented_movie_file(&source)] {
                let read_back = read(&written.unwrap()).unwrap();
                assert_eq!(
                    (read_back.samples, read_back.errors),
                    (source.samples.clone(), Vec::new())
                );
            }
        }
    }
}
