//! Reading the samples of a media segment through the stack that holds its structure
//!
//! The whole of what a caller writes to read a media segment: the movie it
//! continues is handed over first, bytes go in as they arrive, and the samples
//! come out.

use isobmff_boxes::MovieBox;
use isobmff_sample::Sample;
use isobmff_structure::MediaSegmentReader;

/// The samples `segment` carries against `movie`, read off it `cut_length` bytes at a time
pub(crate) fn samples_of(movie: MovieBox, segment: &[u8], cut_length: usize) -> Vec<Sample> {
    let mut reader = MediaSegmentReader::new(movie);
    let mut samples = Vec::new();

    for arriving in segment.chunks(cut_length) {
        reader.handle_input(arriving).unwrap();
        samples.extend(drained(&mut reader));
    }

    reader.finish().unwrap();
    samples.extend(drained(&mut reader));

    samples
}

/// Takes every sample the reader has completed
pub(crate) fn drained(reader: &mut MediaSegmentReader) -> Vec<Sample> {
    let mut samples = Vec::new();
    while let Some(sample) = reader.poll_sample() {
        samples.push(sample);
    }

    samples
}
