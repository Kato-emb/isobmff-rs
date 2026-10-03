//! The samples of a fragmented movie file, written to a sink and read back off a source that seeks

#[cfg(test)]
mod tests {
    use std::io;

    use futures_executor::block_on;
    use futures_util::io::Cursor;
    use isobmff_io::blocking::{Sink, Source};
    use isobmff_sample::Sample;
    use isobmff_sample::movie_fragment_random_access::sync_sample_at;
    use isobmff_structure::{FragmentedDemuxFsm, FragmentedMuxFsm};
    use isobmff_test_support::{
        file_type, fragmented_file_samples, indexed_fragmented_file, presentation_movie,
    };

    /// The samples `fsm` reads off `source` from where it stands to the end of the file
    fn read_to_end(
        fsm: &mut FragmentedDemuxFsm,
        source: &mut Source<io::Cursor<Vec<u8>>>,
    ) -> Vec<Sample> {
        let mut read_back = Vec::new();
        while let Some(wanted) = fsm.wanted_input() {
            let bytes = source.read_at(wanted.offset(), wanted.length()).unwrap();
            let handed = if bytes.is_empty() {
                fsm.finish()
            } else {
                fsm.handle_input(wanted.offset(), bytes)
            };
            read_back.extend(core::iter::from_fn(|| fsm.poll_sample()));
            handed.unwrap();
        }

        read_back
    }

    /// The samples `fsm` reads off the asynchronous `source` from where it stands to the end of the file
    async fn read_to_end_asynchronously(
        fsm: &mut FragmentedDemuxFsm,
        source: &mut isobmff_io::Source<Cursor<Vec<u8>>>,
    ) -> Vec<Sample> {
        let mut read_back = Vec::new();
        while let Some(wanted) = fsm.wanted_input() {
            let bytes = source
                .read_at(wanted.offset(), wanted.length())
                .await
                .unwrap();
            let handed = if bytes.is_empty() {
                fsm.finish()
            } else {
                fsm.handle_input(wanted.offset(), bytes)
            };
            read_back.extend(core::iter::from_fn(|| fsm.poll_sample()));
            handed.unwrap();
        }

        read_back
    }

    /// A mux FSM that has laid down a file of one fragment carrying the samples of the fixture
    fn laid_down() -> FragmentedMuxFsm {
        let mut fsm = FragmentedMuxFsm::new();
        fsm.handle_file_type(file_type()).unwrap();
        fsm.handle_movie(presentation_movie()).unwrap();
        fsm.begin_fragment(1).unwrap();
        for sample in fragmented_file_samples() {
            fsm.handle_sample(sample).unwrap();
        }
        fsm.finish_fragment().unwrap();
        fsm.finish().unwrap();

        fsm
    }

    #[test]
    fn the_samples_written_to_a_sink_are_read_back_off_a_source() {
        let mut mux_fsm = laid_down();
        let mut file = Vec::new();
        let mut sink = Sink::new(&mut file);
        sink.write(core::iter::from_fn(|| mux_fsm.poll_output()))
            .unwrap();
        sink.flush().unwrap();

        let mut source = Source::new(io::Cursor::new(file)).unwrap();

        assert_eq!(
            read_to_end(&mut FragmentedDemuxFsm::new(), &mut source),
            fragmented_file_samples()
        );
    }

    #[test]
    fn the_samples_written_to_an_asynchronous_sink_are_read_back_off_an_asynchronous_source() {
        let mut mux_fsm = laid_down();

        let read_back = block_on(async {
            let mut file = Vec::new();
            let mut sink = isobmff_io::Sink::new(&mut file);
            sink.write(core::iter::from_fn(|| mux_fsm.poll_output()))
                .await
                .unwrap();
            sink.flush().await.unwrap();

            let mut source = isobmff_io::Source::new(Cursor::new(file)).await.unwrap();
            read_to_end_asynchronously(&mut FragmentedDemuxFsm::new(), &mut source).await
        });

        assert_eq!(read_back, fragmented_file_samples());
    }

    #[test]
    fn the_mfra_located_at_the_end_of_the_file_names_the_fragment_a_time_is_read_from() {
        let file = indexed_fragmented_file();
        let second = file.fragment_samples.get(1).unwrap();
        let mut source = Source::new(io::Cursor::new(file.bytes.clone())).unwrap();
        let mut fsm = FragmentedDemuxFsm::new();
        read_to_end(&mut fsm, &mut source);

        let mfra = source
            .locate_movie_fragment_random_access()
            .unwrap()
            .unwrap();
        fsm.resume_at(mfra).unwrap();
        assert_eq!(read_to_end(&mut fsm, &mut source), []);
        let tfra = fsm
            .movie_fragment_random_access()
            .unwrap()
            .tfra()
            .first()
            .unwrap();
        let moof_offset = sync_sample_at(tfra, second.first().unwrap().decode_time())
            .unwrap()
            .moof_offset();
        fsm.resume_at(moof_offset).unwrap();

        assert_eq!(read_to_end(&mut fsm, &mut source), *second);
    }

    #[test]
    fn the_mfra_located_asynchronously_at_the_end_of_the_file_names_the_fragment_a_time_is_read_from()
     {
        let file = indexed_fragmented_file();
        let second = file.fragment_samples.get(1).unwrap();

        let read_back = block_on(async {
            let mut source = isobmff_io::Source::new(Cursor::new(file.bytes.clone()))
                .await
                .unwrap();
            let mut fsm = FragmentedDemuxFsm::new();
            read_to_end_asynchronously(&mut fsm, &mut source).await;

            let mfra = source
                .locate_movie_fragment_random_access()
                .await
                .unwrap()
                .unwrap();
            fsm.resume_at(mfra).unwrap();
            assert_eq!(read_to_end_asynchronously(&mut fsm, &mut source).await, []);
            let tfra = fsm
                .movie_fragment_random_access()
                .unwrap()
                .tfra()
                .first()
                .unwrap();
            let moof_offset = sync_sample_at(tfra, second.first().unwrap().decode_time())
                .unwrap()
                .moof_offset();
            fsm.resume_at(moof_offset).unwrap();

            read_to_end_asynchronously(&mut fsm, &mut source).await
        });

        assert_eq!(read_back, *second);
    }
}
