//! The samples a muxer laid down as a non-fragmented movie file, read back off that file by the demuxer

// Why not `cfg(all(test, feature = "std"))` on the module: the
// `tests_outside_test_module` lint reads the module attribute literally and
// fires on anything but a bare `cfg(test)`.
#![cfg(feature = "std")]

#[cfg(test)]
mod tests {
    use std::io;

    use isobmff::{NonFragmentedDemuxer, NonFragmentedMuxer, Sample};
    use isobmff_test_support::{
        SAMPLE_CHUNKS, file_type, non_fragmented_file_samples, unfragmented_movie,
    };

    #[test]
    fn the_samples_the_muxer_wrote_to_a_sink_are_read_back_off_it_by_the_demuxer() {
        let mut file = Vec::new();
        let mut muxer = NonFragmentedMuxer::new(&mut file);
        let mut samples = non_fragmented_file_samples().into_iter();

        muxer.handle_file_type(file_type()).unwrap();
        muxer.handle_movie(unfragmented_movie()).unwrap();
        for chunk in SAMPLE_CHUNKS {
            muxer.begin_chunk().unwrap();
            for _sample in chunk {
                muxer.handle_sample(samples.next().unwrap()).unwrap();
            }
        }
        muxer.finish().unwrap();

        let read_back: Vec<Sample> = NonFragmentedDemuxer::new(io::Cursor::new(&file))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert_eq!(read_back, non_fragmented_file_samples());
    }
}
