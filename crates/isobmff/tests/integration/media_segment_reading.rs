//! Reading the samples of a media segment through the stack that holds its structure
//!
//! The whole of what a caller writes to read a media segment: the movie it
//! continues is handed over first, the bytes the demuxer wants are read and
//! handed over at their offset, and the samples come out.

use isobmff::MediaSegmentDemuxer;
use isobmff::sample::Sample;
use isobmff_boxes::MovieBox;

/// The samples `segment` carries against `movie`, read off it where the demuxer wants, `cut_length` bytes at a time at most
pub(crate) fn samples_of(movie: MovieBox, segment: &[u8], cut_length: usize) -> Vec<Sample> {
    let mut demuxer = MediaSegmentDemuxer::new(movie).unwrap();
    let mut samples = Vec::new();

    while let Some(wanted) = demuxer.wanted_input() {
        let rest = segment
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
        samples.extend(drained(&mut demuxer));
        handed.unwrap();
    }

    samples
}

/// Takes every sample the reader has completed
pub(crate) fn drained(demuxer: &mut MediaSegmentDemuxer) -> Vec<Sample> {
    let mut samples = Vec::new();
    while let Some(sample) = demuxer.poll_output() {
        samples.push(sample);
    }

    samples
}
