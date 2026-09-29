//! Reading the samples of a non-fragmented movie file through the stack that holds its structure
//!
//! The whole of what a caller writes to read such a file: bytes go in as they
//! arrive, the bytes the reader wants are fetched and handed back, and the
//! samples come out.

use core::ops::Range;

use isobmff_sample::Sample;
use isobmff_structure::NonFragmentedDemuxFsm;

/// The bytes of `file` lying at `extent`
pub(crate) fn fetched<'file>(file: &'file [u8], extent: &Range<u64>) -> &'file [u8] {
    file.get(usize::try_from(extent.start).unwrap()..usize::try_from(extent.end).unwrap())
        .unwrap()
}

/// Hands `file` over in order, `cut_length` bytes at a time, and returns the samples that completed
pub(crate) fn handed_over_in_order(
    demux_fsm: &mut NonFragmentedDemuxFsm,
    file: &[u8],
    cut_length: usize,
) -> Vec<Sample> {
    let mut samples = Vec::new();

    for arriving in file.chunks(cut_length) {
        demux_fsm.handle_input(arriving).unwrap();
        while let Some(sample) = demux_fsm.poll_sample() {
            samples.push(sample);
        }
    }

    samples
}

/// The samples `file` carries, read off it `cut_length` bytes at a time and then off the bytes it wants fetched
pub(crate) fn samples_of(file: &[u8], cut_length: usize) -> Vec<Sample> {
    let mut demux_fsm = NonFragmentedDemuxFsm::new();
    let mut samples = handed_over_in_order(&mut demux_fsm, file, cut_length);

    while let Some(wanted) = demux_fsm.wanted_extent() {
        demux_fsm
            .handle_data(wanted.start, fetched(file, &wanted))
            .unwrap();
        while let Some(sample) = demux_fsm.poll_sample() {
            samples.push(sample);
        }
    }
    demux_fsm.finish().unwrap();
    while let Some(sample) = demux_fsm.poll_sample() {
        samples.push(sample);
    }

    samples
}
