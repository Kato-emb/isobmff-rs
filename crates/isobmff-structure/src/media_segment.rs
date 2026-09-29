//! [`MediaSegmentDemuxFsm`] and [`MediaSegmentMuxFsm`], a media segment read and written through the layers this crate holds, ISO/IEC 14496-12 §8.16

mod demux;
mod mux;
mod structure;

pub use demux::MediaSegmentDemuxFsm;
pub use mux::MediaSegmentMuxFsm;

use structure::{MediaSegmentDisposition, MediaSegmentStructure};

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use isobmff_boxes::{MovieBox, SampleFlags, TrackExtendsBox};
    use isobmff_sample::Sample;
    use isobmff_test_support::fragmented_movie;

    use super::MediaSegmentMuxFsm;

    /// Movie of one track the segments continue, whose defaults a `trex` states
    pub(super) fn movie() -> MovieBox {
        fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO))
    }

    /// One sample of track 1, the first of its fragment
    pub(super) fn sample() -> Sample {
        Sample::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec())
    }

    /// A segment of one fragment carrying [`sample`], with no brands
    pub(super) fn segment_of_one_sample() -> Vec<u8> {
        let mut mux_fsm = MediaSegmentMuxFsm::new();
        let mut segment = Vec::new();

        mux_fsm.begin_fragment(1).unwrap();
        mux_fsm.handle_sample(sample()).unwrap();
        mux_fsm.finish_fragment().unwrap();
        mux_fsm.finish().unwrap();
        while let Some(written) = mux_fsm.poll_output() {
            segment.extend_from_slice(&written);
        }

        segment
    }
}
