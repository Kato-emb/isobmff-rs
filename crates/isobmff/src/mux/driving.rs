//! The media segment muxer, driven over one sample, for the unit tests of this crate

use alloc::vec::Vec;

use isobmff_boxes::{MovieBox, SampleFlags, TrackExtendsBox};
use isobmff_test_support::fragmented_movie;

use crate::MediaSegmentMuxer;
use crate::sample::{Sample, SampleProperties};

/// Movie of one track the segments continue, whose defaults a `trex` states
pub(crate) fn movie() -> MovieBox {
    fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO))
}

/// One sample of track 1, the first of its fragment
pub(crate) fn sample() -> Sample {
    Sample::new(
        SampleProperties {
            track_id: 1,
            decode_time: 0,
            sample_duration: 1_024,
            sample_composition_time_offset: 0,
            sample_flags: SampleFlags::ZERO,
            sample_description_index: 1,
        },
        b"SAMP".to_vec(),
    )
}

/// A segment of one fragment carrying [`sample`], with no brands
pub(crate) fn segment_of_one_sample() -> Vec<u8> {
    let mut muxer = MediaSegmentMuxer::new(&movie()).unwrap();
    let mut segment = Vec::new();

    muxer.begin_fragment(1).unwrap();
    muxer.handle_sample(sample()).unwrap();
    muxer.finish_fragment().unwrap();
    muxer.finish().unwrap();
    while let Some(written) = muxer.poll_output() {
        segment.extend_from_slice(&written);
    }

    segment
}
