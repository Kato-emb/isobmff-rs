//! A file read with the demuxer where it asks and written again with a muxer as it is read, as a movie file or as a fragmented movie file
//!
//! The example hands each sample the demuxer lends to [`MovieFileMuxer`],
//! or with `--fragmented` to [`FragmentedMovieFileMuxer`], which ends a
//! fragment where one of the file did and where the next sample would pass
//! the limits of the muxer, and writes what the muxer holds after each sample.
//! A movie file then takes the header of its `mdat`, known once every sample
//! is in it, written over the first. Memory is bounded by the limits of
//! [`DemuxerConfig`] and [`MuxerConfig`], whatever the length of the file.
//!
//! The output is a new file: an output that exists already is refused, so
//! the input is never written over, whatever path names it.
//!
//! `cargo run --example remux -- <input> <output> [--fragmented]`

use std::error::Error;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use isobmff::demuxer::Event;
use isobmff::muxer::{self, MovieDescription, TrackDescription};
use isobmff::{
    Demuxer, DemuxerConfig, FragmentedMovieFileMuxer, MovieFileMuxer, MuxerConfig, structure_data,
};

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args().skip(1);
    let usage = "usage: remux <input> <output> [--fragmented]";
    let (input, output) = (
        arguments.next().ok_or(usage)?,
        arguments.next().ok_or(usage)?,
    );
    let fragmented = arguments.next().as_deref() == Some("--fragmented");
    run(Path::new(&input), Path::new(&output), fragmented)
}

/// Writes the samples of the file at `input` to a new file at `output`, as a fragmented movie file where `fragmented`, and as a movie file otherwise
fn run(input: &Path, output: &Path, fragmented: bool) -> Result<(), Box<dyn Error>> {
    let (input, output) = (
        std::fs::File::open(input)?,
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)?,
    );
    if fragmented {
        fragmented_movie_file(input, output)
    } else {
        movie_file(input, output)
    }
}

/// Writes the samples of `input` to `output` as a movie file: its `ftyp`, one `mdat` and the `moov` that declares them
fn movie_file(
    mut input: impl Read + Seek,
    mut output: impl Write + Seek,
) -> Result<(), Box<dyn Error>> {
    let mut demuxer = Demuxer::new(DemuxerConfig::default());
    let (mut buffer, mut out) = (vec![0; 64 * 1024], Vec::new());
    let (mut file_type, mut muxer) = (None, None);
    loop {
        while let Some(event) = demuxer.poll_event() {
            match event {
                Event::FileType(ftyp) => file_type = Some(ftyp),
                // Describe the movie and its tracks to the muxer, which writes the opening of the file
                Event::Movie(moov) => {
                    let file_type = file_type.take().ok_or("a moov before any ftyp")?;
                    let mut movie_muxer = MovieFileMuxer::new(
                        MovieDescription::from(&*moov),
                        MuxerConfig::default(),
                        file_type,
                    )?;
                    for trak in &moov.trak {
                        movie_muxer.add_track(TrackDescription::from(trak))?;
                        demuxer.subscribe_track(trak.tkhd.track_id);
                    }
                    muxer = Some(movie_muxer);
                }
                // Hand the sample over as it is lent, and write what the muxer holds
                Event::Sample(sample) => {
                    let muxer = muxer.as_mut().ok_or("a sample before any moov")?;
                    muxer.handle_sample(sample.properties, sample.bytes)?;
                    muxer.poll_output(&mut out);
                    output.write_all(&out)?;
                    out.clear();
                }
                Event::Error(error) => return Err(error.into()),
                _ => {}
            }
        }
        let Some(required) = demuxer.required_input() else {
            break;
        };
        input.seek(SeekFrom::Start(required.position))?;
        match input.read(&mut buffer)? {
            0 => demuxer.finish(),
            read => demuxer.handle_input(required.position, &buffer[..read]),
        }
    }
    // The `moov` follows the samples; the header of the `mdat` is written over the first
    let mut muxer = muxer.ok_or("no moov")?;
    let header = muxer.finish()?;
    muxer.poll_output(&mut out);
    output.write_all(&out)?;
    output.seek(SeekFrom::Start(header.offset))?;
    output.write_all(&header.bytes)?;
    Ok(())
}

/// Writes the samples of `input` to `output` as a fragmented movie file, a fragment ending where one of `input` does and where the next sample would pass the limits of the muxer
fn fragmented_movie_file(
    mut input: impl Read + Seek,
    mut output: impl Write,
) -> Result<(), Box<dyn Error>> {
    let mut demuxer = Demuxer::new(DemuxerConfig::default());
    let (mut buffer, mut out) = (vec![0; 64 * 1024], Vec::new());
    let (mut file_type, mut muxer) = (None, None);
    loop {
        while let Some(event) = demuxer.poll_event() {
            match event {
                Event::FileType(ftyp) => file_type = Some(ftyp),
                Event::Movie(moov) => {
                    let file_type = file_type.take().ok_or("a moov before any ftyp")?;
                    let mut fragmented_muxer = FragmentedMovieFileMuxer::new(
                        MovieDescription::from(&*moov),
                        MuxerConfig::default(),
                        file_type,
                    )?;
                    for trak in &moov.trak {
                        fragmented_muxer.add_track(TrackDescription::from(trak))?;
                        demuxer.subscribe_track(trak.tkhd.track_id);
                    }
                    muxer = Some(fragmented_muxer);
                }
                // A fragment of the file ends where the next begins
                Event::MovieFragment(_) => {
                    if let Some(muxer) = muxer.as_mut() {
                        muxer.finish_fragment()?;
                    }
                }
                Event::Sample(sample) => {
                    let muxer = muxer.as_mut().ok_or("a sample before any moov")?;
                    match muxer.handle_sample(sample.properties, sample.bytes) {
                        // The fragment is full: end it, and hand the sample over again
                        Err(
                            muxer::Error::BufferFull { .. }
                            | muxer::Error::StructureData(
                                structure_data::Error::SampleCountOverLimit { .. },
                            ),
                        ) => {
                            muxer.poll_output(&mut out);
                            muxer.finish_fragment()?;
                            muxer.poll_output(&mut out);
                            output.write_all(&out)?;
                            out.clear();
                            muxer.handle_sample(sample.properties, sample.bytes)?;
                        }
                        result => result?,
                    }
                    muxer.poll_output(&mut out);
                    output.write_all(&out)?;
                    out.clear();
                }
                Event::Error(error) => return Err(error.into()),
                _ => {}
            }
        }
        let Some(required) = demuxer.required_input() else {
            break;
        };
        input.seek(SeekFrom::Start(required.position))?;
        match input.read(&mut buffer)? {
            0 => demuxer.finish(),
            read => demuxer.handle_input(required.position, &buffer[..read]),
        }
    }
    let mut muxer = muxer.ok_or("no moov")?;
    muxer.finish()?;
    muxer.poll_output(&mut out);
    output.write_all(&out)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use isobmff::demuxer::Event;
    use isobmff::{Demuxer, DemuxerConfig, SampleProperties};

    use super::{fragmented_movie_file, movie_file};

    /// Every sample of `file` with its bytes, in file order, as the demuxer lends them
    fn samples_of(file: &[u8]) -> Vec<(SampleProperties, Vec<u8>)> {
        let mut demuxer = Demuxer::new(DemuxerConfig::default());
        let mut samples = Vec::new();
        loop {
            while let Some(event) = demuxer.poll_event() {
                match event {
                    Event::Movie(moov) => {
                        for trak in &moov.trak {
                            demuxer.subscribe_track(trak.tkhd.track_id);
                        }
                    }
                    Event::Sample(sample) => {
                        samples.push((sample.properties, sample.bytes.to_vec()));
                    }
                    _ => {}
                }
            }
            let Some(required) = demuxer.required_input() else {
                break;
            };
            match file.get(usize::try_from(required.position).unwrap()..) {
                Some(bytes) if !bytes.is_empty() => demuxer.handle_input(required.position, bytes),
                _ => demuxer.finish(),
            }
        }
        samples
    }

    #[test]
    fn a_file_written_again_as_a_movie_file_or_in_fragments_holds_the_same_samples() {
        for file in [
            include_bytes!("../crates/isobmff/tests/fixtures/movie_moov_last.mp4").as_slice(),
            include_bytes!("../crates/isobmff/tests/fixtures/fragmented.mp4"),
        ] {
            let mut as_movie_file = Cursor::new(Vec::new());
            movie_file(Cursor::new(file), &mut as_movie_file).unwrap();
            let mut in_fragments = Vec::new();
            fragmented_movie_file(Cursor::new(file), &mut in_fragments).unwrap();
            assert_eq!(
                (
                    samples_of(as_movie_file.get_ref()),
                    samples_of(&in_fragments)
                ),
                (samples_of(file), samples_of(file))
            );
        }
    }

    #[test]
    fn an_output_that_already_exists_is_refused_and_the_input_is_left_as_it_was() {
        let fixture = include_bytes!("../crates/isobmff/tests/fixtures/movie_moov_last.mp4");
        let directory =
            std::env::temp_dir().join(format!("isobmff-remux-output-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let (input, link, other) = (
            directory.join("input.mp4"),
            directory.join("link.mp4"),
            directory.join("other.mp4"),
        );
        std::fs::write(&input, fixture).unwrap();
        std::fs::hard_link(&input, &link).unwrap();
        std::fs::write(&other, b"other").unwrap();
        for output in [&input, &link, &other] {
            for fragmented in [false, true] {
                let refused = super::run(&input, output, fragmented).is_err();
                assert_eq!(
                    (
                        refused,
                        std::fs::read(&input).unwrap(),
                        std::fs::read(&other).unwrap()
                    ),
                    (true, fixture.to_vec(), b"other".to_vec())
                );
            }
        }
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
