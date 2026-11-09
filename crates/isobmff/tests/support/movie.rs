//! The `moov` of a file and the samples it declares, which the tests of reading and writing sample tables share

use isobmff::SampleLocation;
use isobmff::framing::{BoxReader, Event, FramerConfig};
use isobmff::structure_data::{Error, StructureDataReader};
use isobmff_boxes::MovieBox;
use isobmff_core::{BoxDecode, BoxDefinition};

/// The most samples the tests resolve from one `moov`
pub(crate) const SAMPLE_COUNT_LIMIT: u64 = 1 << 20;

/// The `moov` of `file`, read with the framing reader
pub(crate) fn movie_of(file: &[u8]) -> Option<MovieBox> {
    let mut reader = BoxReader::new(FramerConfig::new(64 * 1024 * 1024));
    let mut movie = None;
    loop {
        while let Some(event) = reader.poll_event() {
            if let Event::Gathered {
                box_type, payload, ..
            } = event
                && box_type == MovieBox::BOX_TYPE
            {
                movie = Some(MovieBox::decode_payload(payload).unwrap());
            }
        }
        let Some(required) = reader.required_input() else {
            break;
        };
        match file.get(usize::try_from(required.position).unwrap()..) {
            Some(bytes) if !bytes.is_empty() => reader.handle_input(required.position, bytes),
            _ => reader.finish(),
        }
    }
    movie
}

/// The location of each sample the tracks of `movie` declare, in the order of their file offsets
pub(crate) fn locations(movie: &MovieBox) -> Result<Vec<SampleLocation>, Error> {
    StructureDataReader::new(SAMPLE_COUNT_LIMIT)
        .handle_movie(movie)
        .map(Iterator::collect)
}
