//! A file read with the demuxer, its samples counted by track
//!
//! The example drives [`Demuxer`] the way a caller that seeks does: it hands
//! over the file from where the demuxer asks, subscribes to every track once
//! the `moov` is read, and takes the bytes of each sample it lends.
//!
//! `cargo run --example demux -- <input>`

use std::collections::BTreeMap;

use isobmff::boxes::{FileTypeBox, MovieBox};
use isobmff::demuxer::{Error, Event};
use isobmff::{Demuxer, DemuxerConfig, SampleProperties};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = std::env::args().nth(1).ok_or("usage: demux <input>")?;
    let source = read(&std::fs::read(input)?).ok_or("no ftyp or no moov")?;
    for (track_id, (samples, length)) in by_track(&source) {
        println!("track {track_id}: {samples} samples, {length} bytes");
    }
    for error in &source.errors {
        println!("error: {error}");
    }
    Ok(())
}

/// The number of samples of each track of `source`, and of their bytes
fn by_track(source: &Read) -> BTreeMap<u32, (usize, usize)> {
    let mut tracks = BTreeMap::<u32, (usize, usize)>::new();
    for (properties, bytes) in &source.samples {
        let (samples, length) = tracks.entry(properties.track_id).or_default();
        *samples = samples.saturating_add(1);
        *length = length.saturating_add(bytes.len());
    }
    tracks
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
    /// What the demuxer could not read
    errors: Vec<Error>,
}

/// Reads `file` where the demuxer asks, subscribing to every track, or returns `None` where it holds no `ftyp` or no `moov`
#[must_use]
fn read(file: &[u8]) -> Option<Read> {
    let mut demuxer = Demuxer::new(DemuxerConfig::default());
    let (mut file_type, mut movie) = (None, None);
    let (mut samples, mut errors) = (Vec::new(), Vec::new());
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
                Event::Sample(sample) => samples.push((sample.properties, sample.bytes.to_vec())),
                Event::Error(error) => errors.push(error),
                Event::SegmentType(_)
                | Event::MovieFragment(_)
                | Event::SampleLocation(_)
                | Event::Box { .. }
                | _ => {}
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
        errors,
    })
}

#[cfg(test)]
mod tests {
    use super::{by_track, read};

    #[test]
    fn every_sample_of_a_fixture_is_lent_whether_its_moov_comes_first_or_last() {
        for file in [
            include_bytes!("../crates/isobmff/tests/fixtures/movie_moov_first.mp4").as_slice(),
            include_bytes!("../crates/isobmff/tests/fixtures/movie_moov_last.mp4"),
        ] {
            let source = read(file).unwrap();
            assert_eq!(
                (by_track(&source), source.errors),
                ([(1, (12, 17_615)), (2, (58, 4_932))].into(), Vec::new())
            );
        }
    }
}
