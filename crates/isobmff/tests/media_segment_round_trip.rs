//! The samples a muxer laid down as a media segment, read back off that segment by the demuxer

// Why not `cfg(all(test, feature = "std"))` on the module: the
// `tests_outside_test_module` lint reads the module attribute literally and
// fires on anything but a bare `cfg(test)`.
#![cfg(feature = "std")]

#[cfg(test)]
mod tests {
    use std::io;

    use isobmff::{MediaSegmentDemuxer, MediaSegmentMuxer, Sample};
    use isobmff_test_support::{presentation_movie, segment_file_samples, segment_type};

    #[test]
    fn the_samples_the_muxer_wrote_to_a_sink_are_read_back_off_it_by_the_demuxer() {
        let mut segment = Vec::new();
        let mut muxer = MediaSegmentMuxer::new(&mut segment);

        muxer.handle_segment_type(segment_type()).unwrap();
        muxer.begin_fragment(1).unwrap();
        for sample in segment_file_samples() {
            muxer.handle_sample(sample).unwrap();
        }
        muxer.finish_fragment().unwrap();
        muxer.finish().unwrap();

        let read_back: Vec<Sample> =
            MediaSegmentDemuxer::new(io::Cursor::new(&segment), presentation_movie())
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();

        assert_eq!(read_back, segment_file_samples());
    }
}
