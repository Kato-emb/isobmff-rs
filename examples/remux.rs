//! A file read with the demuxer where it asks and written again with a muxer as it is read, as a movie file or as a fragmented movie file
//!
//! The muxer takes the samples of a track in decoding order, which need not
//! be the order their bytes lie in the file, ISO/IEC 14496-12 §8.7.5. The
//! example takes the location of each sample of the `moov` or a `moof` from
//! the demuxer, orders them by decoding time, reads the bytes of each where
//! its location says, and hands it to [`MovieFileMuxer`], or with
//! `--fragmented` to [`FragmentedMovieFileMuxer`], which ends a fragment
//! where one of the file did and where the next sample would pass the limits
//! of the muxer. Samples of different tracks are ordered by their decoding
//! times in seconds: how to interleave the tracks is the example's choice.
//! It writes what the muxer holds after each sample. A movie file then takes
//! the header of its `mdat`, known once every sample is in it, written over
//! the first. Memory is bounded by the limits of [`DemuxerConfig`] and
//! [`MuxerConfig`], whatever the length of the file.
//!
//! The output is a new file: an output that exists already is refused, so
//! the input is never written over, whatever path names it.
//!
//! `cargo run --example remux -- <input> <output> [--fragmented]`

use std::error::Error;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use isobmff::demuxer::Event;
use isobmff::muxer::{self, MovieParameters, TrackParameters};
use isobmff::{
    Demuxer, DemuxerConfig, FragmentedMovieFileMuxer, MovieFileMuxer, MuxerConfig, SampleLocation,
    SampleProperties, structure_data,
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

/// Hands each sample of `held` to `handle` with its bytes, read from `input`, in decoding order, the samples of tracks of different `timescales` ordered by their decoding times in seconds
fn hand_over(
    held: &mut Vec<SampleLocation>,
    timescales: &[(u32, u32)],
    input: &mut (impl Read + Seek),
    mut handle: impl FnMut(SampleProperties, &[u8]) -> Result<(), Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    let timescale = |location: &SampleLocation| {
        timescales
            .iter()
            .find(|&&(track_id, _)| track_id == location.properties.track_id)
            .map_or(1, |&(_, timescale)| u128::from(timescale.max(1)))
    };
    // Why not compare the decoding times as they are: those of tracks of
    // different timescales count different units. Each is scaled by the
    // timescale of the other, a u64 by a u32, which a u128 holds.
    held.sort_by(|first, second| {
        (u128::from(first.properties.decoding_time) * timescale(second))
            .cmp(&(u128::from(second.properties.decoding_time) * timescale(first)))
    });
    let limit = DemuxerConfig::default().sample_size_limit;
    let mut bytes = Vec::new();
    for location in held.drain(..) {
        if u64::from(location.size) > limit {
            return Err(format!(
                "a sample of track {} is {} bytes, over the limit of {limit}",
                location.properties.track_id, location.size
            )
            .into());
        }
        bytes.resize(usize::try_from(location.size)?, 0);
        input.seek(SeekFrom::Start(location.offset))?;
        input.read_exact(&mut bytes)?;
        handle(location.properties, &bytes)?;
    }
    Ok(())
}

/// The `track_ID` and media `timescale` of each track of `moov`
fn timescales_of(moov: &isobmff::boxes::MovieBox) -> Vec<(u32, u32)> {
    moov.trak
        .iter()
        .map(|trak| (trak.tkhd.track_id, trak.mdia.mdhd.timescale))
        .collect()
}

/// Writes the samples of `input` to `output` as a movie file: its `ftyp`, one `mdat` and the `moov` that declares them
fn movie_file(
    mut input: impl Read + Seek,
    mut output: impl Write + Seek,
) -> Result<(), Box<dyn Error>> {
    let mut demuxer = Demuxer::new(DemuxerConfig::default());
    let (mut buffer, mut out) = (vec![0; 64 * 1024], Vec::new());
    let (mut file_type, mut muxer, mut timescales, mut held) = (None, None, Vec::new(), Vec::new());
    let mut write =
        |muxer: &mut MovieFileMuxer, properties, bytes: &[u8]| -> Result<(), Box<dyn Error>> {
            muxer.handle_sample(properties, bytes)?;
            muxer.poll_output(&mut out);
            output.write_all(&out)?;
            out.clear();
            Ok(())
        };
    loop {
        while let Some(event) = demuxer.poll_event() {
            match event {
                Event::FileType(ftyp) => file_type = Some(ftyp),
                Event::Movie(moov) => {
                    let file_type = file_type.take().ok_or("a moov before any ftyp")?;
                    let mut movie_muxer = MovieFileMuxer::new(
                        MovieParameters::from(&*moov),
                        MuxerConfig::default(),
                        file_type,
                    )?;
                    for trak in &moov.trak {
                        movie_muxer.add_track(TrackParameters::from(trak))?;
                    }
                    timescales = timescales_of(&moov);
                    muxer = Some(movie_muxer);
                }
                Event::MovieFragment(_) => {
                    let muxer = muxer.as_mut().ok_or("a moof before any moov")?;
                    hand_over(&mut held, &timescales, &mut input, |properties, bytes| {
                        write(muxer, properties, bytes)
                    })?;
                }
                Event::SampleLocation(location) => held.push(location),
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
    hand_over(&mut held, &timescales, &mut input, |properties, bytes| {
        write(&mut muxer, properties, bytes)
    })?;
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
    let (mut file_type, mut muxer, mut timescales, mut held) = (None, None, Vec::new(), Vec::new());
    let mut write = |muxer: &mut FragmentedMovieFileMuxer,
                     properties,
                     bytes: &[u8]|
     -> Result<(), Box<dyn Error>> {
        match muxer.handle_sample(properties, bytes) {
            Err(
                muxer::Error::BufferFull { .. }
                | muxer::Error::StructureData(structure_data::Error::SampleCountOverLimit { .. }),
            ) => {
                muxer.poll_output(&mut out);
                muxer.finish_fragment()?;
                muxer.poll_output(&mut out);
                output.write_all(&out)?;
                out.clear();
                muxer.handle_sample(properties, bytes)?;
            }
            result => result?,
        }
        muxer.poll_output(&mut out);
        output.write_all(&out)?;
        out.clear();
        Ok(())
    };
    loop {
        while let Some(event) = demuxer.poll_event() {
            match event {
                Event::FileType(ftyp) => file_type = Some(ftyp),
                Event::Movie(moov) => {
                    let file_type = file_type.take().ok_or("a moov before any ftyp")?;
                    let mut fragmented_muxer = FragmentedMovieFileMuxer::new(
                        MovieParameters::from(&*moov),
                        MuxerConfig::default(),
                        file_type,
                    )?;
                    for trak in &moov.trak {
                        fragmented_muxer.add_track(TrackParameters::from(trak))?;
                    }
                    timescales = timescales_of(&moov);
                    muxer = Some(fragmented_muxer);
                }
                Event::MovieFragment(_) => {
                    let muxer = muxer.as_mut().ok_or("a moof before any moov")?;
                    hand_over(&mut held, &timescales, &mut input, |properties, bytes| {
                        write(muxer, properties, bytes)
                    })?;
                    muxer.finish_fragment()?;
                }
                Event::SampleLocation(location) => held.push(location),
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
    hand_over(&mut held, &timescales, &mut input, |properties, bytes| {
        write(&mut muxer, properties, bytes)
    })?;
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

    /// Every sample of `file` with its bytes, as the demuxer lends them, track by track and in decoding order within each
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
        samples.sort_by_key(|(properties, _)| (properties.track_id, properties.decoding_time));
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

    #[test]
    fn a_file_whose_samples_lie_out_of_decoding_order_is_written_again_with_them_in_it() {
        let file = include_bytes!("../crates/isobmff/tests/fixtures/chunks_out_of_order.mp4");
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

    #[test]
    fn a_movie_whose_tables_disagree_is_not_written() {
        use isobmff::BoxType;
        use isobmff::demuxer::Error;
        use isobmff::structure_data;

        let refused = |result: Result<(), Box<dyn std::error::Error>>| {
            result.unwrap_err().downcast_ref::<Error>().copied()
        };
        /// File offset of the `moov` of `movie_moov_last.mp4`, which the fixtures change in place
        const MOOV_OFFSET: u64 = 22595;
        for (file, box_type, table_sample_count) in [
            (
                include_bytes!("../crates/isobmff/tests/fixtures/mismatch_tables.mp4").as_slice(),
                b"stts",
                10,
            ),
            (
                include_bytes!("../crates/isobmff/tests/fixtures/mismatch_chunks.mp4"),
                b"stsc",
                24,
            ),
        ] {
            let error = Error::StructureData {
                offset: MOOV_OFFSET,
                box_type: BoxType::compact(*b"moov"),
                source: structure_data::Error::SampleCountMismatch {
                    track_id: 1,
                    box_type: BoxType::compact(*box_type),
                    sample_count: 12,
                    table_sample_count,
                },
            };
            assert_eq!(
                [
                    refused(movie_file(Cursor::new(file), Cursor::new(Vec::new()))),
                    refused(fragmented_movie_file(Cursor::new(file), Vec::new())),
                ],
                [Some(error); 2]
            );
        }
    }
}
