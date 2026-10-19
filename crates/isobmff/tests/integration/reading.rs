//! Reading the samples of a file through the stack that holds its structure
//!
//! The whole of what a caller writes to read a fragmented file: the bytes the
//! demuxer wants are read and handed over at their offset, and the samples
//! come out.

use isobmff::MovieDemuxer;
use isobmff::sample::Sample;

/// The samples `file` carries, read off it where the demuxer wants, `cut_length` bytes at a time at most
pub(crate) fn samples_of(file: &[u8], cut_length: usize) -> Vec<Sample> {
    read_on(&mut MovieDemuxer::new(), file, cut_length)
}

/// The samples `demuxer` reads off `file` where it wants, `cut_length` bytes at a time at most, until it wants no more
pub(crate) fn read_on(demuxer: &mut MovieDemuxer, file: &[u8], cut_length: usize) -> Vec<Sample> {
    let mut samples = Vec::new();

    while let Some(wanted) = demuxer.wanted_input() {
        let rest = file
            .get(usize::try_from(wanted.offset()).unwrap()..)
            .unwrap_or_default();
        let length = wanted
            .length()
            .map_or(cut_length, |length| usize::try_from(length).unwrap());
        let read = rest.get(..length.min(cut_length)).unwrap_or(rest);
        let handed = if read.is_empty() {
            demuxer.finish()
        } else {
            demuxer.handle_input(wanted.offset(), read)
        };
        samples.extend(drained(demuxer));
        handed.unwrap();
    }

    samples
}

/// Takes every sample the reader has completed
pub(crate) fn drained(demuxer: &mut MovieDemuxer) -> Vec<Sample> {
    let mut samples = Vec::new();
    while let Some(sample) = demuxer.poll_output() {
        samples.push(sample);
    }

    samples
}
