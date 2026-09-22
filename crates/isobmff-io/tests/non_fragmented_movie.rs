//! The samples of a non-fragmented movie file, read off a source that seeks wherever the movie lies and read back off one a muxer laid down

#[cfg(test)]
mod tests {
    use std::io;

    use isobmff_io::blocking::{NonFragmentedDemuxer, NonFragmentedMuxer};
    use isobmff_sample::Sample;
    use isobmff_test_support::{
        SAMPLE_CHUNKS, file_type, non_fragmented_file, non_fragmented_file_samples,
        unfragmented_movie,
    };

    #[test]
    fn a_source_that_seeks_has_every_sample_read_off_it_wherever_the_movie_lies() {
        for movie_first in [true, false] {
            let file = non_fragmented_file(&SAMPLE_CHUNKS, movie_first);

            let read_back: Vec<Sample> = NonFragmentedDemuxer::new(io::Cursor::new(file))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();

            assert_eq!(read_back, non_fragmented_file_samples());
        }
    }

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
