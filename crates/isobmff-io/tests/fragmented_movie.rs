//! The samples of a fragmented movie file, read off a source that seeks and read back off one a muxer laid down

#[cfg(test)]
mod tests {
    use std::io;

    use isobmff_io::blocking::{FragmentedDemuxer, FragmentedMuxer};
    use isobmff_sample::Sample;
    use isobmff_test_support::{
        file_type, fragmented_file_samples, fragmented_file_with_samples, presentation_movie,
    };

    #[test]
    fn a_source_that_seeks_has_every_sample_read_off_it() {
        let read_back: Vec<Sample> =
            FragmentedDemuxer::new(io::Cursor::new(fragmented_file_with_samples()))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();

        assert_eq!(read_back, fragmented_file_samples());
    }

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
