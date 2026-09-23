//! The samples of a fragmented movie file, read off a source that seeks and read back off one a muxer laid down

#[cfg(test)]
mod tests {
    use std::io;

    use futures_executor::block_on;
    use futures_util::io::Cursor;
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

    #[test]
    fn an_asynchronous_source_that_seeks_has_every_sample_read_off_it() {
        let read_back = block_on(async {
            let mut demuxer =
                isobmff_io::FragmentedDemuxer::new(Cursor::new(fragmented_file_with_samples()))
                    .await
                    .unwrap();
            let mut read_back = Vec::new();
            while let Some(sample) = demuxer.next().await {
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
            let mut muxer = isobmff_io::FragmentedMuxer::new(&mut file);
            muxer.handle_file_type(file_type()).await.unwrap();
            muxer.handle_movie(presentation_movie()).await.unwrap();
            muxer.begin_fragment(1).await.unwrap();
            for sample in fragmented_file_samples() {
                muxer.handle_sample(sample).await.unwrap();
            }
            muxer.finish_fragment().await.unwrap();
            muxer.finish().await.unwrap();

            let mut demuxer = isobmff_io::FragmentedDemuxer::new(Cursor::new(&file))
                .await
                .unwrap();
            let mut read_back = Vec::new();
            while let Some(sample) = demuxer.next().await {
                read_back.push(sample.unwrap());
            }

            read_back
        });

        assert_eq!(read_back, fragmented_file_samples());
    }
}
