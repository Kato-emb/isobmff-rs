//! The samples a muxer laid down as a fragmented movie file, read back off that file by the demuxer

// Why not `cfg(all(test, feature = "std"))` on the module: the
// `tests_outside_test_module` lint reads the module attribute literally and
// fires on anything but a bare `cfg(test)`.
#![cfg(feature = "std")]

#[cfg(test)]
mod tests {
    use std::io;

    use isobmff::{FragmentedDemuxer, FragmentedMuxer, Sample};
    use isobmff_test_support::{file_type, fragmented_file_samples, presentation_movie};

    #[test]
    fn the_samples_the_muxer_wrote_to_a_sink_are_read_back_off_it_by_the_demuxer() {
        let mut file = Vec::new();
        let mut muxer = FragmentedMuxer::new(&mut file);

        muxer.handle_file_type(file_type()).unwrap();
        muxer.handle_movie(presentation_movie()).unwrap();
        muxer.begin_fragment(1).unwrap();
        for sample in fragmented_file_samples() {
            muxer.handle_sample(sample).unwrap();
        }
        muxer.finish_fragment().unwrap();
        muxer.finish().unwrap();

        let read_back: Vec<Sample> = FragmentedDemuxer::new(io::Cursor::new(&file))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert_eq!(read_back, fragmented_file_samples());
    }
}
