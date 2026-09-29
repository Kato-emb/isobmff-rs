//! Reading the samples of a media segment through the stack that holds its structure
//!
//! The whole of what a caller writes to read a media segment: the movie it
//! continues is handed over first, bytes go in as they arrive, and the samples
//! come out.

use isobmff_boxes::MovieBox;
use isobmff_sample::Sample;
use isobmff_structure::MediaSegmentDemuxFsm;

/// The samples `segment` carries against `movie`, read off it `cut_length` bytes at a time
pub(crate) fn samples_of(movie: MovieBox, segment: &[u8], cut_length: usize) -> Vec<Sample> {
    let mut demux_fsm = MediaSegmentDemuxFsm::new(movie);
    let mut samples = Vec::new();

    for arriving in segment.chunks(cut_length) {
        demux_fsm.handle_input(arriving).unwrap();
        samples.extend(drained(&mut demux_fsm));
    }

    demux_fsm.finish().unwrap();
    samples.extend(drained(&mut demux_fsm));

    samples
}

/// Takes every sample the reader has completed
pub(crate) fn drained(demux_fsm: &mut MediaSegmentDemuxFsm) -> Vec<Sample> {
    let mut samples = Vec::new();
    while let Some(sample) = demux_fsm.poll_sample() {
        samples.push(sample);
    }

    samples
}
