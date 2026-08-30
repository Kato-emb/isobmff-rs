//! Reading the samples of a file through the layer that holds its layout
//!
//! The whole of what a caller writes to read a fragmented file: bytes go in as
//! they arrive, and the samples come out.

use isobmff::{FragmentedReader, Sample};

/// The samples `file` carries, read off it `cut_length` bytes at a time
pub(crate) fn samples_of(file: &[u8], cut_length: usize) -> Vec<Sample> {
    let mut reader = FragmentedReader::new();
    let mut samples = Vec::new();

    for arriving in file.chunks(cut_length) {
        reader.handle_input(arriving).unwrap();
        while let Some(sample) = reader.poll_sample() {
            samples.push(sample);
        }
    }

    reader.finish().unwrap();
    while let Some(sample) = reader.poll_sample() {
        samples.push(sample);
    }

    samples
}
