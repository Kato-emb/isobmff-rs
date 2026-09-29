//! The samples of a fragmented movie file, read off a source that seeks and read back off one a muxer laid down

#[cfg(test)]
mod tests {
    use std::io;

    use futures_executor::block_on;
    use futures_util::io::Cursor;
    use isobmff_io::blocking::{DemuxDriver, MuxDriver};
    use isobmff_sample::Sample;
    use isobmff_sample::movie_fragment_random_access::sync_sample_at;
    use isobmff_structure::{FragmentedDemuxFsm, FragmentedMuxFsm};
    use isobmff_test_support::{
        file_type, fragmented_file_samples, fragmented_file_with_samples, indexed_fragmented_file,
        presentation_movie,
    };

    #[test]
    fn a_source_that_seeks_has_every_sample_read_off_it() {
        let read_back: Vec<Sample> = DemuxDriver::new(
            io::Cursor::new(fragmented_file_with_samples()),
            FragmentedDemuxFsm::new(),
        )
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

        assert_eq!(read_back, fragmented_file_samples());
    }

    #[test]
    fn the_samples_the_muxer_wrote_to_a_sink_are_read_back_off_it_by_the_demuxer() {
        let mut file = Vec::new();
        let mut mux_driver = MuxDriver::new(&mut file, FragmentedMuxFsm::new());
        let mux_fsm = mux_driver.fsm_mut();

        mux_fsm.handle_file_type(file_type()).unwrap();
        mux_fsm.handle_movie(presentation_movie()).unwrap();
        mux_fsm.begin_fragment(1).unwrap();
        for sample in fragmented_file_samples() {
            mux_fsm.handle_sample(sample).unwrap();
        }
        mux_fsm.finish_fragment().unwrap();
        mux_fsm.finish().unwrap();
        mux_driver.flush().unwrap();

        let read_back: Vec<Sample> =
            DemuxDriver::new(io::Cursor::new(&file), FragmentedDemuxFsm::new())
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();

        assert_eq!(read_back, fragmented_file_samples());
    }

    #[test]
    fn an_asynchronous_source_that_seeks_has_every_sample_read_off_it() {
        let read_back = block_on(async {
            let mut driver = isobmff_io::DemuxDriver::new(
                Cursor::new(fragmented_file_with_samples()),
                FragmentedDemuxFsm::new(),
            )
            .await
            .unwrap();
            let mut read_back = Vec::new();
            while let Some(sample) = driver.next().await {
                read_back.push(sample.unwrap());
            }

            read_back
        });

        assert_eq!(read_back, fragmented_file_samples());
    }

    #[test]
    fn the_samples_the_asynchronous_muxer_wrote_to_a_sink_are_read_back_off_it_by_the_demuxer() {
        let mut file = Vec::new();

        let read_back = block_on(async {
            let mut mux_driver = isobmff_io::MuxDriver::new(&mut file, FragmentedMuxFsm::new());
            let mux_fsm = mux_driver.fsm_mut();
            mux_fsm.handle_file_type(file_type()).unwrap();
            mux_fsm.handle_movie(presentation_movie()).unwrap();
            mux_fsm.begin_fragment(1).unwrap();
            for sample in fragmented_file_samples() {
                mux_fsm.handle_sample(sample).unwrap();
            }
            mux_fsm.finish_fragment().unwrap();
            mux_fsm.finish().unwrap();
            mux_driver.flush().await.unwrap();

            let mut driver =
                isobmff_io::DemuxDriver::new(Cursor::new(&file), FragmentedDemuxFsm::new())
                    .await
                    .unwrap();
            let mut read_back = Vec::new();
            while let Some(sample) = driver.next().await {
                read_back.push(sample.unwrap());
            }

            read_back
        });

        assert_eq!(read_back, fragmented_file_samples());
    }

    #[test]
    fn the_mfra_found_at_the_end_of_the_file_names_the_fragment_a_time_is_read_from() {
        let file = indexed_fragmented_file();
        let second = file.fragment_samples.get(1).unwrap();
        let mut driver = DemuxDriver::new(
            io::Cursor::new(file.bytes.clone()),
            FragmentedDemuxFsm::new(),
        )
        .unwrap();
        driver.next().unwrap().unwrap();

        let mfra = driver
            .locate_movie_fragment_random_access()
            .unwrap()
            .unwrap();
        driver.fsm_mut().resume_at(mfra).unwrap();
        assert!(driver.next().is_none());
        let tfra = driver
            .fsm()
            .movie_fragment_random_access()
            .unwrap()
            .tfra()
            .first()
            .unwrap();
        let moof_offset = sync_sample_at(tfra, second.first().unwrap().decode_time())
            .unwrap()
            .moof_offset();
        driver.fsm_mut().resume_at(moof_offset).unwrap();

        assert_eq!(driver.collect::<Result<Vec<_>, _>>().unwrap(), *second);
    }

    #[test]
    fn the_mfra_an_asynchronous_demux_driver_finds_at_the_end_of_the_file_names_the_fragment_a_time_is_read_from()
     {
        let file = indexed_fragmented_file();
        let second = file.fragment_samples.get(1).unwrap();

        let read_back = block_on(async {
            let mut driver = isobmff_io::DemuxDriver::new(
                Cursor::new(file.bytes.clone()),
                FragmentedDemuxFsm::new(),
            )
            .await
            .unwrap();
            driver.next().await.unwrap().unwrap();

            let mfra = driver
                .locate_movie_fragment_random_access()
                .await
                .unwrap()
                .unwrap();
            driver.fsm_mut().resume_at(mfra).unwrap();
            assert!(driver.next().await.is_none());
            let tfra = driver
                .fsm()
                .movie_fragment_random_access()
                .unwrap()
                .tfra()
                .first()
                .unwrap();
            let moof_offset = sync_sample_at(tfra, second.first().unwrap().decode_time())
                .unwrap()
                .moof_offset();
            driver.fsm_mut().resume_at(moof_offset).unwrap();
            let mut read_back = Vec::new();
            while let Some(sample) = driver.next().await {
                read_back.push(sample.unwrap());
            }

            read_back
        });

        assert_eq!(read_back, *second);
    }
}
