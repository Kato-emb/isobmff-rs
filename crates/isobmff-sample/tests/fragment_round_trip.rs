//! The samples a movie fragment is written from resolve back out of it as the extents they were laid down at

#[cfg(test)]
mod tests {
    use isobmff_boxes::TrackExtendsBox;
    use isobmff_core::BoxEncode as _;
    use isobmff_sample::movie_fragment::sample_extents;
    use isobmff_sample::{MovieFragmentWriter, Sample, SampleExtent, TrackDecodeTimes};
    use isobmff_test_support::fragmented_movie;

    /// Where the `moof` of this test lies in the file
    const MOOF_START: u64 = 1_000;

    /// Bytes the header of the `mdat` beside the fragment occupies
    const MEDIA_DATA_HEADER_LEN: u64 = 8;

    #[test]
    fn the_samples_a_fragment_is_written_from_resolve_back_out_of_it() {
        let movie = fragmented_movie(TrackExtendsBox::new(1, 1, 0, 0, 0));
        let samples = [
            Sample::new(1, 0, 3_000, 0, 0x0200_0000, 1, b"AAAAAAAA".to_vec()),
            Sample::new(1, 3_000, 3_000, 8, 0x0101_0000, 1, b"BBBB".to_vec()),
            Sample::new(1, 6_000, 1_500, -8, 0x0101_0000, 1, b"CC".to_vec()),
        ];
        let mut writer = MovieFragmentWriter::new();

        writer.begin_fragment(1).unwrap();
        for sample in &samples {
            writer.handle_sample(sample.clone()).unwrap();
        }
        let (movie_fragment, media_data) = writer.finish_fragment().unwrap();

        let data_start = MOOF_START + movie_fragment.encoded_len() + MEDIA_DATA_HEADER_LEN;
        let mut decode_times = TrackDecodeTimes::new();
        let extents: Vec<SampleExtent> =
            sample_extents(&movie_fragment, &movie, MOOF_START, &mut decode_times)
                .unwrap()
                .map(Result::unwrap)
                .collect();

        assert_eq!(media_data, b"AAAAAAAABBBBCC");
        assert_eq!(decode_times.decode_time(1), 7_500);
        assert_eq!(
            extents,
            [
                SampleExtent::new(
                    1,
                    0,
                    3_000,
                    0,
                    0x0200_0000,
                    1,
                    1,
                    data_start..data_start + 8
                ),
                SampleExtent::new(
                    1,
                    3_000,
                    3_000,
                    8,
                    0x0101_0000,
                    1,
                    1,
                    data_start + 8..data_start + 12
                ),
                SampleExtent::new(
                    1,
                    6_000,
                    1_500,
                    -8,
                    0x0101_0000,
                    1,
                    1,
                    data_start + 12..data_start + 14
                ),
            ]
        );
    }
}
