//! The samples of a media segment, written to a sink and read back off a source that seeks against the movie it continues

#[cfg(test)]
mod tests {
    use std::io;

    use futures_executor::block_on;
    use futures_util::io::Cursor;
    use isobmff_io::blocking::{Sink, Source};
    use isobmff_structure::{MediaSegmentDemuxFsm, MediaSegmentMuxFsm};
    use isobmff_test_support::{presentation_movie, segment_file_samples, segment_type};

    /// A mux FSM that has laid down a segment of one fragment carrying the samples of the fixture
    fn laid_down() -> MediaSegmentMuxFsm {
        let mut fsm = MediaSegmentMuxFsm::new();
        fsm.handle_segment_type(segment_type()).unwrap();
        fsm.begin_fragment(1).unwrap();
        for sample in segment_file_samples() {
            fsm.handle_sample(sample).unwrap();
        }
        fsm.finish_fragment().unwrap();
        fsm.finish().unwrap();

        fsm
    }

    #[test]
    fn the_samples_written_to_a_sink_are_read_back_off_a_source() {
        let mut mux_fsm = laid_down();
        let mut segment = Vec::new();
        let mut sink = Sink::new(&mut segment);
        sink.write(core::iter::from_fn(|| mux_fsm.poll_output()))
            .unwrap();
        sink.flush().unwrap();

        let mut source = Source::new(io::Cursor::new(segment)).unwrap();
        let mut fsm = MediaSegmentDemuxFsm::new(presentation_movie());
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

        assert_eq!(read_back, segment_file_samples());
    }

    #[test]
    fn the_samples_written_to_an_asynchronous_sink_are_read_back_off_an_asynchronous_source() {
        let mut mux_fsm = laid_down();

        let read_back = block_on(async {
            let mut segment = Vec::new();
            let mut sink = isobmff_io::Sink::new(&mut segment);
            sink.write(core::iter::from_fn(|| mux_fsm.poll_output()))
                .await
                .unwrap();
            sink.flush().await.unwrap();

            let mut source = isobmff_io::Source::new(Cursor::new(segment)).await.unwrap();
            let mut fsm = MediaSegmentDemuxFsm::new(presentation_movie());
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

        assert_eq!(read_back, segment_file_samples());
    }
}
