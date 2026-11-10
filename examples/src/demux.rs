//! [`read`], which reads a whole file with the demuxer

use isobmff::demuxer::{Error, Event};
use isobmff::{Demuxer, DemuxerConfig, SampleProperties};
use isobmff_boxes::{FileTypeBox, MovieBox};

/// What the demuxer reads of a file: its `ftyp`, its `moov`, and every sample of its tracks with its bytes, in file order
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Read {
    /// The `ftyp` of the file
    pub file_type: FileTypeBox,
    /// The `moov` of the file
    pub movie: MovieBox,
    /// What the structure-data declares of each sample, with its bytes
    pub samples: Vec<(SampleProperties, Vec<u8>)>,
    /// Index in `samples` of the first sample read after each `moof`, in the
    /// order the boxes come
    pub fragment_starts: Vec<usize>,
    /// What the demuxer could not read
    pub errors: Vec<Error>,
}

/// Reads `file` where the demuxer asks, subscribing to every track, or returns `None` where it holds no `ftyp` or no `moov`
#[must_use]
pub fn read(file: &[u8]) -> Option<Read> {
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
