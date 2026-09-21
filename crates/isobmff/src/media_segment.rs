//! [`MediaSegmentReader`] and [`MediaSegmentWriter`], a media segment read and written through the layers this crate holds, ISO/IEC 14496-12 §8.16

#[cfg(feature = "std")]
mod demuxer;
#[cfg(feature = "std")]
mod muxer;
mod reader;
mod structure;
mod writer;

#[cfg(feature = "std")]
pub use demuxer::MediaSegmentDemuxer;
#[cfg(feature = "std")]
pub use muxer::MediaSegmentMuxer;
pub use reader::MediaSegmentReader;
pub use writer::MediaSegmentWriter;

use structure::{MediaSegmentDisposition, MediaSegmentStructure};

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use isobmff_boxes::{MovieBox, TrackExtendsBox};
    use isobmff_sample::Sample;
    use isobmff_test_support::fragmented_movie;

    use super::MediaSegmentWriter;

    /// Movie of one track the segments continue, whose defaults a `trex` states
    pub(super) fn movie() -> MovieBox {
        fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, 0))
    }

    /// One sample of track 1, the first of its fragment
    pub(super) fn sample() -> Sample {
        Sample::new(1, 0, 1_024, 0, 0, 1, b"SAMP".to_vec())
    }

    /// A segment of one fragment carrying [`sample`], with no brands
    pub(super) fn segment_of_one_sample() -> Vec<u8> {
        let mut writer = MediaSegmentWriter::new();
        let mut segment = Vec::new();

        writer.begin_fragment(1).unwrap();
        writer.handle_sample(sample()).unwrap();
        writer.finish_fragment().unwrap();
        writer.finish().unwrap();
        while let Some(written) = writer.poll_output() {
            segment.extend_from_slice(&written);
        }

        segment
    }
}
