//! The samples of a non-fragmented movie file, written to a sink with the movie after its media data and read back off a source that seeks

#[cfg(test)]
mod tests {
    use std::io;

    use futures_executor::block_on;
    use futures_util::io::Cursor;
    use isobmff_io::blocking::{Sink, Source};
    use isobmff_structure::{NonFragmentedDemuxFsm, NonFragmentedMuxFsm};
    use isobmff_test_support::{
        SAMPLE_CHUNKS, file_type, non_fragmented_file_samples, unfragmented_movie,
    };

    /// A mux FSM that has laid down the samples of the fixture in its chunks
    fn laid_down() -> NonFragmentedMuxFsm {
        let mut fsm = NonFragmentedMuxFsm::new();
        let mut samples = non_fragmented_file_samples().into_iter();
        fsm.handle_file_type(file_type()).unwrap();
        fsm.handle_movie(unfragmented_movie()).unwrap();
        for chunk in SAMPLE_CHUNKS {
            fsm.begin_chunk().unwrap();
            for _sample in chunk {
                fsm.handle_sample(samples.next().unwrap()).unwrap();
            }
        }
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
        let mut fsm = NonFragmentedDemuxFsm::new();
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

        assert_eq!(read_back, non_fragmented_file_samples());
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
            let mut fsm = NonFragmentedDemuxFsm::new();
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
        });

        assert_eq!(read_back, non_fragmented_file_samples());
    }
}
