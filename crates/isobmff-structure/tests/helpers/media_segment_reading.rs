//! Reading the samples of a media segment through the stack that holds its structure
//!
//! The whole of what a caller writes to read a media segment: the movie it
//! continues is handed over first, the bytes the demux FSM wants are read and
//! handed over at their offset, and the samples come out.

use isobmff_boxes::MovieBox;
use isobmff_sample::Sample;
use isobmff_structure::MediaSegmentDemuxFsm;

/// The samples `segment` carries against `movie`, read off it where the demux FSM wants, `cut_length` bytes at a time at most
pub(crate) fn samples_of(movie: MovieBox, segment: &[u8], cut_length: usize) -> Vec<Sample> {
    let mut demux_fsm = MediaSegmentDemuxFsm::new(movie).unwrap();
    let mut samples = Vec::new();

    while let Some(wanted) = demux_fsm.wanted_input() {
        let rest = segment
            .get(usize::try_from(wanted.offset()).unwrap()..)
            .unwrap_or_default();
        let length = wanted
            .length()
            .map_or(cut_length, |length| usize::try_from(length).unwrap());
        let read = rest.get(..length.min(cut_length)).unwrap_or(rest);
        let handed = if read.is_empty() {
            demux_fsm.finish()
        } else {
            demux_fsm.handle_input(wanted.offset(), read)
        };
        samples.extend(drained(&mut demux_fsm));
        handed.unwrap();
    }

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
