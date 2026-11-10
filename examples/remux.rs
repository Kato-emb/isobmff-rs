//! A file read with the demuxer and written again with a muxer, as a movie file or as a fragmented movie file
//!
//! The example reads every sample of a file with [`Demuxer`], then hands
//! them to [`MovieFileMuxer`], or with `--fragmented` to
//! [`FragmentedMovieFileMuxer`], which ends a fragment where one of the file
//! did, and where the next sample would pass the limits of the muxer.
//!
//! `cargo run --example remux -- <input> <output> [--fragmented]`

use std::error::Error;

use isobmff::boxes::{FileTypeBox, MovieBox};
use isobmff::demuxer::Event;
use isobmff::muxer::{self, MovieDescription, TrackDescription};
use isobmff::{
    Demuxer, DemuxerConfig, FragmentedMovieFileMuxer, MovieFileMuxer, MuxerConfig,
    SampleProperties, structure_data,
};

/// What the structure-data declares of each sample of a file, with its bytes, in file order
type Samples = Vec<(SampleProperties, Vec<u8>)>;

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args().skip(1);
    let usage = "usage: remux <input> <output> [--fragmented]";
    let (input, output) = (
        arguments.next().ok_or(usage)?,
        arguments.next().ok_or(usage)?,
    );
    let fragmented = arguments.next().as_deref() == Some("--fragmented");
    let (file_type, movie, samples, fragment_starts) =
        read(&std::fs::read(input)?).ok_or("no ftyp or no moov, or a box that does not read")?;
    let written = if fragmented {
        fragmented_movie_file(&file_type, &movie, &samples, &fragment_starts)?
    } else {
        movie_file(&file_type, &movie, &samples)?
    };
    std::fs::write(output, written)?;
    Ok(())
}

/// Reads `file` where the demuxer asks, every track subscribed to: its `ftyp`, its `moov`, every sample with its bytes in file order, and the index of the first sample after each `moof`, or `None` where it lacks one of the boxes or holds what the demuxer cannot read
fn read(file: &[u8]) -> Option<(FileTypeBox, MovieBox, Samples, Vec<usize>)> {
    let mut demuxer = Demuxer::new(DemuxerConfig::default());
    let (mut file_type, mut movie, mut readable) = (None, None, true);
    let (mut samples, mut fragment_starts) = (Vec::new(), Vec::new());
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
                Event::Error(_) => readable = false,
                _ => {}
            }
        }
        let Some(required) = demuxer.required_input() else {
            break;
        };
        match file.get(usize::try_from(required.position).ok()?..) {
            Some(bytes) if !bytes.is_empty() => demuxer.handle_input(required.position, bytes),
            _ => demuxer.finish(),
        }
    }
    readable.then_some((file_type?, movie?, samples, fragment_starts))
}

/// Returns the movie file of `samples`, of the tracks of `movie`: its `ftyp`, one `mdat` and the `moov` that declares them
fn movie_file(
    file_type: &FileTypeBox,
    movie: &MovieBox,
    samples: &Samples,
) -> Result<Vec<u8>, muxer::Error> {
    let mut muxer = MovieFileMuxer::new(
        MovieDescription::from(movie),
        MuxerConfig::default(),
        file_type.clone(),
    )?;
    for trak in &movie.trak {
        muxer.add_track(TrackDescription::from(trak))?;
    }
    let mut written = Vec::new();
    for (properties, bytes) in samples {
        muxer.handle_sample(*properties, bytes)?;
        muxer.poll_output(&mut written);
    }
    let header = muxer.finish()?;
    muxer.poll_output(&mut written);
    // The length of the `mdat` is known only now: write its header over the first
    let first = usize::try_from(header.offset).unwrap_or(usize::MAX);
    written[first..first + header.bytes.len()].copy_from_slice(&header.bytes);
    Ok(written)
}

/// Returns the fragmented movie file of `samples`, of the tracks of `movie`, a fragment ending at each of `fragment_starts` and where the next sample would pass the limits of the muxer
fn fragmented_movie_file(
    file_type: &FileTypeBox,
    movie: &MovieBox,
    samples: &Samples,
    fragment_starts: &[usize],
) -> Result<Vec<u8>, muxer::Error> {
    let mut muxer = FragmentedMovieFileMuxer::new(
        MovieDescription::from(movie),
        MuxerConfig::default(),
        file_type.clone(),
    )?;
    for trak in &movie.trak {
        muxer.add_track(TrackDescription::from(trak))?;
    }
    let mut written = Vec::new();
    for (index, (properties, bytes)) in samples.iter().enumerate() {
        if fragment_starts.binary_search(&index).is_ok() {
            muxer.finish_fragment()?;
        }
        match muxer.handle_sample(*properties, bytes) {
            // The fragment is full: end it, and hand the sample over again
            Err(
                muxer::Error::BufferFull { .. }
                | muxer::Error::StructureData(structure_data::Error::SampleCountOverLimit { .. }),
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
            let (file_type, movie, samples, fragment_starts) = read(file).unwrap();
            for written in [
                movie_file(&file_type, &movie, &samples),
                fragmented_movie_file(&file_type, &movie, &samples, &fragment_starts),
            ] {
                let (_, _, read_back, _) = read(&written.unwrap()).unwrap();
                assert_eq!(read_back, samples);
            }
        }
    }
}
