//! Reading the samples of a file through the stack that holds its structure
//!
//! The whole of what a caller writes to read a fragmented file: the bytes the
//! demux FSM wants are read and handed over at their offset, and the samples
//! come out.

use isobmff_sample::Sample;
use isobmff_structure::MovieDemuxFsm;

/// The samples `file` carries, read off it where the demux FSM wants, `cut_length` bytes at a time at most
pub(crate) fn samples_of(file: &[u8], cut_length: usize) -> Vec<Sample> {
    read_on(&mut MovieDemuxFsm::new(), file, cut_length)
}

/// The samples `demux_fsm` reads off `file` where it wants, `cut_length` bytes at a time at most, until it wants no more
pub(crate) fn read_on(
    demux_fsm: &mut MovieDemuxFsm,
    file: &[u8],
    cut_length: usize,
) -> Vec<Sample> {
    let mut samples = Vec::new();

    while let Some(wanted) = demux_fsm.wanted_input() {
        let rest = file
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
        samples.extend(drained(demux_fsm));
        handed.unwrap();
    }

    samples
}

/// Takes every sample the reader has completed
pub(crate) fn drained(demux_fsm: &mut MovieDemuxFsm) -> Vec<Sample> {
    let mut samples = Vec::new();
    while let Some(sample) = demux_fsm.poll_sample() {
        samples.push(sample);
    }

    samples
}
