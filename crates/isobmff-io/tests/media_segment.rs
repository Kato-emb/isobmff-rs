//! The samples of a media segment, read off a source that seeks against the movie it continues and read back off one a muxer laid down

#[cfg(test)]
mod tests {
    use std::io;

    use futures_executor::block_on;
    use futures_util::io::Cursor;
    use isobmff_io::blocking::{MediaSegmentDemuxer, MediaSegmentMuxer};
    use isobmff_sample::Sample;
    use isobmff_test_support::{
        presentation_movie, segment_file_samples, segment_file_with_samples, segment_type,
    };

    #[test]
    fn a_source_that_seeks_has_every_sample_read_off_it() {
        let read_back: Vec<Sample> = MediaSegmentDemuxer::new(
            io::Cursor::new(segment_file_with_samples()),
            presentation_movie(),
        )
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

        assert_eq!(read_back, segment_file_samples());
    }

    #[test]
    fn the_samples_the_muxer_wrote_to_a_sink_are_read_back_off_it_by_the_demuxer() {
        let mut segment = Vec::new();
        let mut muxer = MediaSegmentMuxer::new(&mut segment);

        muxer.handle_segment_type(segment_type()).unwrap();
        muxer.begin_fragment(1).unwrap();
        for sample in segment_file_samples() {
            muxer.handle_sample(sample).unwrap();
        }
        muxer.finish_fragment().unwrap();
        muxer.finish().unwrap();

        let read_back: Vec<Sample> =
            MediaSegmentDemuxer::new(io::Cursor::new(&segment), presentation_movie())
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();

        assert_eq!(read_back, segment_file_samples());
    }

    #[test]
    fn an_asynchronous_source_that_seeks_has_every_sample_read_off_it() {
        let read_back = block_on(async {
            let mut demuxer = isobmff_io::MediaSegmentDemuxer::new(
                Cursor::new(segment_file_with_samples()),
                presentation_movie(),
            )
            .await
            .unwrap();
            let mut read_back = Vec::new();
            while let Some(sample) = demuxer.next().await {
                read_back.push(sample.unwrap());
            }

            read_back
        });

        assert_eq!(read_back, segment_file_samples());
    }

    #[test]
    fn the_samples_the_asynchronous_muxer_wrote_to_a_sink_are_read_back_off_it_by_the_demuxer() {
        let mut segment = Vec::new();

        let read_back = block_on(async {
            let mut muxer = isobmff_io::MediaSegmentMuxer::new(&mut segment);
            muxer.handle_segment_type(segment_type()).await.unwrap();
            muxer.begin_fragment(1).await.unwrap();
            for sample in segment_file_samples() {
                muxer.handle_sample(sample).await.unwrap();
            }
            muxer.finish_fragment().await.unwrap();
            muxer.finish().await.unwrap();

            let mut demuxer =
                isobmff_io::MediaSegmentDemuxer::new(Cursor::new(&segment), presentation_movie())
                    .await
                    .unwrap();
            let mut read_back = Vec::new();
            while let Some(sample) = demuxer.next().await {
                read_back.push(sample.unwrap());
            }

            read_back
        });

        assert_eq!(read_back, segment_file_samples());
    }
}
