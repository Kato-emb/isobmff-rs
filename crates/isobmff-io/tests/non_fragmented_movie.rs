//! The samples of a non-fragmented movie file, read off a source that seeks wherever the movie lies and read back off one a muxer laid down

#[cfg(test)]
mod tests {
    use std::io;

    use futures_executor::block_on;
    use futures_util::io::Cursor;
    use isobmff_boxes::{HeaderDuration, MovieBox};
    use isobmff_io::blocking::{DemuxDriver, MuxDriver};
    use isobmff_sample::Sample;
    use isobmff_structure::{NonFragmentedDemuxFsm, NonFragmentedMuxFsm};
    use isobmff_test_support::{
        SAMPLE_CHUNKS, SAMPLE_DURATION, file_type, non_fragmented_file,
        non_fragmented_file_samples, unfragmented_movie,
    };

    #[test]
    fn a_source_that_seeks_has_every_sample_read_off_it_wherever_the_movie_lies() {
        for movie_first in [true, false] {
            let file = non_fragmented_file(&SAMPLE_CHUNKS, movie_first);

            let read_back: Vec<Sample> =
                DemuxDriver::new(io::Cursor::new(file), NonFragmentedDemuxFsm::new())
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap();

            assert_eq!(read_back, non_fragmented_file_samples());
        }
    }

    #[test]
    fn the_samples_the_muxer_wrote_to_a_sink_are_read_back_off_it_by_the_demuxer() {
        let mut file = Vec::new();
        let mut mux_driver = MuxDriver::new(&mut file, NonFragmentedMuxFsm::new());
        let mux_fsm = mux_driver.fsm_mut();
        let mut samples = non_fragmented_file_samples().into_iter();

        mux_fsm.handle_file_type(file_type()).unwrap();
        mux_fsm.handle_movie(unfragmented_movie()).unwrap();
        for chunk in SAMPLE_CHUNKS {
            mux_fsm.begin_chunk().unwrap();
            for _sample in chunk {
                mux_fsm.handle_sample(samples.next().unwrap()).unwrap();
            }
        }
        mux_fsm.finish().unwrap();
        mux_driver.flush().unwrap();

        let read_back: Vec<Sample> =
            DemuxDriver::new(io::Cursor::new(&file), NonFragmentedDemuxFsm::new())
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();

        assert_eq!(read_back, non_fragmented_file_samples());
    }

    #[test]
    fn a_movie_the_muxer_laid_down_from_a_template_of_no_duration_is_read_back_with_a_header_lasting_its_samples()
     {
        let mut file = Vec::new();
        let mut mux_driver = MuxDriver::new(&mut file, NonFragmentedMuxFsm::new());
        let mux_fsm = mux_driver.fsm_mut();
        let samples = non_fragmented_file_samples();
        let lasting = samples.len() as u64 * u64::from(SAMPLE_DURATION);

        mux_fsm.handle_movie(unfragmented_movie()).unwrap();
        mux_fsm.begin_chunk().unwrap();
        for sample in samples {
            mux_fsm.handle_sample(sample).unwrap();
        }
        mux_fsm.finish().unwrap();
        mux_driver.flush().unwrap();

        let mut driver =
            DemuxDriver::new(io::Cursor::new(&file), NonFragmentedDemuxFsm::new()).unwrap();
        for sample in &mut driver {
            sample.unwrap();
        }

        assert_eq!(
            driver.fsm().movie().map(MovieBox::mvhd),
            Some(
                &unfragmented_movie()
                    .mvhd()
                    .clone()
                    .with_duration(HeaderDuration::new(lasting).unwrap())
            )
        );
    }

    #[test]
    fn an_asynchronous_source_that_seeks_has_every_sample_read_off_it_wherever_the_movie_lies() {
        for movie_first in [true, false] {
            let file = non_fragmented_file(&SAMPLE_CHUNKS, movie_first);

            let read_back = block_on(async {
                let mut demuxer = isobmff_io::NonFragmentedDemuxer::new(Cursor::new(file))
                    .await
                    .unwrap();
                let mut read_back = Vec::new();
                while let Some(sample) = demuxer.next().await {
                    read_back.push(sample.unwrap());
                }

                read_back
            });

            assert_eq!(read_back, non_fragmented_file_samples());
        }
    }

    #[test]
    fn the_samples_the_asynchronous_muxer_wrote_to_a_sink_are_read_back_off_it_by_the_demuxer() {
        let mut file = Vec::new();
        let mut samples = non_fragmented_file_samples().into_iter();

        let read_back = block_on(async {
            let mut muxer = isobmff_io::NonFragmentedMuxer::new(&mut file);
            muxer.handle_file_type(file_type()).await.unwrap();
            muxer.handle_movie(unfragmented_movie()).await.unwrap();
            for chunk in SAMPLE_CHUNKS {
                muxer.begin_chunk().await.unwrap();
                for _sample in chunk {
                    muxer.handle_sample(samples.next().unwrap()).await.unwrap();
                }
            }
            muxer.finish().await.unwrap();

            let mut demuxer = isobmff_io::NonFragmentedDemuxer::new(Cursor::new(&file))
                .await
                .unwrap();
            let mut read_back = Vec::new();
            while let Some(sample) = demuxer.next().await {
                read_back.push(sample.unwrap());
            }

            read_back
        });

        assert_eq!(read_back, non_fragmented_file_samples());
    }
}
