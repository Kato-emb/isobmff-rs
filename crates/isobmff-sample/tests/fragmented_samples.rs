//! The samples a movie fragment declares come out of the resolver and the reader in turn

#[cfg(test)]
mod tests {
    use isobmff_boxes::{
        MovieFragmentBox, MovieFragmentHeaderBox, TrackExtendsBox, TrackFragmentBox,
        TrackFragmentHeaderBox, TrackFragmentHeaderFlags, TrackRunBox, TrackRunSample,
    };
    use isobmff_sample::movie_fragment::sample_extents;
    use isobmff_sample::{Sample, SampleReader, TrackDecodeTimes};
    use isobmff_test_support::{MEDIA_DATA, fragmented_movie};

    /// Where the `moof` of these tests lies in the file
    const MOOF_START: u64 = 1_000;

    /// How far that `moof` reaches
    const MOOF_LEN: u64 = 200;

    /// Fragment of `sample_count` samples of track 1, their data just past the `moof` and its `mdat` header
    fn movie_fragment(sample_count: u32) -> MovieFragmentBox {
        let rows = (0..sample_count)
            .map(|_| TrackRunSample::new(None, None, None, None))
            .collect();
        let track_fragment = TrackFragmentBox::new(
            TrackFragmentHeaderBox::new(
                TrackFragmentHeaderFlags::DEFAULT_BASE_IS_MOOF,
                1,
                None,
                None,
                None,
                None,
                None,
            ),
            None,
            vec![TrackRunBox::new(Some(i32::try_from(MOOF_LEN + 8).unwrap()), None, rows).unwrap()],
        );

        MovieFragmentBox::new(MovieFragmentHeaderBox::new(1), vec![track_fragment])
    }

    /// Sample of track 1 as the `trex` of the movie settles it, carrying `data`
    fn sample(decode_time: u64, data: &[u8]) -> Sample {
        Sample::new(1, decode_time, 3_000, 0, 0x0200_0000, 1, data.to_vec())
    }

    #[test]
    fn the_samples_a_fragment_declares_are_read_out_of_the_media_data_that_follows_it() {
        let movie = fragmented_movie(TrackExtendsBox::new(1, 1, 3_000, 16, 0x0200_0000));
        let mut decode_times = TrackDecodeTimes::new();
        let mut reader = SampleReader::new();
        let mut samples = Vec::new();

        for (fragment, moof_start) in [(0, MOOF_START), (1, MOOF_START + MOOF_LEN + 8 + 64)] {
            let extents =
                sample_extents(&movie_fragment(4), &movie, moof_start, &mut decode_times).unwrap();
            assert_eq!(decode_times.decode_time(1), (fragment + 1) * 12_000);
            for extent in extents {
                reader.handle_sample_extent(extent.unwrap()).unwrap();
            }

            let data_start = moof_start + MOOF_LEN + 8;
            reader.handle_data(data_start, &MEDIA_DATA[..40]).unwrap();
            reader
                .handle_data(data_start + 40, &MEDIA_DATA[40..])
                .unwrap();
            while let Some(sample) = reader.poll_sample() {
                samples.push(sample);
            }

            assert_eq!(reader.wanted_extent(), None, "fragment {fragment}");
        }
        reader.finish().unwrap();

        assert_eq!(
            samples,
            (0..8)
                .map(|index| sample(index * 3_000, &MEDIA_DATA[..16]))
                .collect::<Vec<_>>()
        );
    }
}
