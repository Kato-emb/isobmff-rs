//! [`FragmentedReader`] and [`FragmentedWriter`], a fragmented movie file read and written through the layers this crate holds, ISO/IEC 14496-12 Annex A.8

#[cfg(feature = "std")]
mod demuxer;
#[cfg(feature = "std")]
mod muxer;
mod reader;
mod structure;
mod writer;

#[cfg(feature = "std")]
pub use demuxer::FragmentedDemuxer;
#[cfg(feature = "std")]
pub use muxer::FragmentedMuxer;
pub use reader::FragmentedReader;
pub use writer::FragmentedWriter;

use structure::{FragmentedDisposition, FragmentedStructure};

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use isobmff_boxes::TrackExtendsBox;
    use isobmff_sample::Sample;
    use isobmff_test_support::fragmented_movie;

    use super::FragmentedWriter;

    /// One sample of track 1, the first of its fragment
    pub(super) fn sample() -> Sample {
        Sample::new(1, 0, 1_024, 0, 0, 1, b"SAMP".to_vec())
    }

    /// A file of one fragment carrying [`sample`], with no brands
    pub(super) fn file_of_one_sample() -> Vec<u8> {
        let mut writer = FragmentedWriter::new();
        let mut file = Vec::new();

        writer
            .handle_movie(fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, 0)))
            .unwrap();
        writer.begin_fragment(1).unwrap();
        writer.handle_sample(sample()).unwrap();
        writer.finish_fragment().unwrap();
        writer.finish().unwrap();
        while let Some(written) = writer.poll_output() {
            file.extend_from_slice(&written);
        }

        file
    }
}
