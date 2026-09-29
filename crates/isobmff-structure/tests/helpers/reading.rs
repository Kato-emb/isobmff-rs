//! Reading the samples of a file through the stack that holds its structure
//!
//! The whole of what a caller writes to read a fragmented file: bytes go in as
//! they arrive, and the samples come out.

use isobmff_sample::Sample;
use isobmff_structure::FragmentedDemuxFsm;

/// The samples `file` carries, read off it `cut_length` bytes at a time
pub(crate) fn samples_of(file: &[u8], cut_length: usize) -> Vec<Sample> {
    let mut demux_fsm = FragmentedDemuxFsm::new();
    let mut samples = Vec::new();

    for arriving in file.chunks(cut_length) {
        demux_fsm.handle_input(arriving).unwrap();
        samples.extend(drained(&mut demux_fsm));
    }

    demux_fsm.finish().unwrap();
    samples.extend(drained(&mut demux_fsm));

    samples
}

/// Takes every sample the reader has completed
pub(crate) fn drained(demux_fsm: &mut FragmentedDemuxFsm) -> Vec<Sample> {
    let mut samples = Vec::new();
    while let Some(sample) = demux_fsm.poll_sample() {
        samples.push(sample);
    }

    samples
}
