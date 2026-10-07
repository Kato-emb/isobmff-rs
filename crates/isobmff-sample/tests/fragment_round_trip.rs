//! The samples a movie fragment is written from resolve back out of it as the extents they were laid down at

#[cfg(test)]
mod tests {
    use isobmff_boxes::{
        DegradationPriorityEntry, IsLeading, PaddingBitsEntry, SampleDependencyTypeEntry,
        SampleDependsOn, SampleFlags, SampleHasRedundancy, SampleIsDependedOn, TrackExtendsBox,
    };
    use isobmff_core::BoxEncode as _;
    use isobmff_sample::movie_fragment::sample_extents;
    use isobmff_sample::{
        MovieFragmentWriter, Sample, SampleExtent, SampleProperties, TrackDecodeTimes,
    };
    use isobmff_test_support::fragmented_movie;

    /// Where the `moof` of this test lies in the file
    const MOOF_START: u64 = 1_000;

    /// Bytes the header of the `mdat` beside the fragment occupies
    const MEDIA_DATA_HEADER_LEN: u64 = 8;

    #[test]
    fn the_samples_a_fragment_is_written_from_resolve_back_out_of_it() {
        let independent = SampleFlags::new(
            SampleDependencyTypeEntry::new(
                IsLeading::Unknown,
                SampleDependsOn::DoesNotDependOnOthers,
                SampleIsDependedOn::Unknown,
                SampleHasRedundancy::Unknown,
            ),
            PaddingBitsEntry::default(),
            false,
            DegradationPriorityEntry::default(),
        );
        let dependent = SampleFlags::new(
            SampleDependencyTypeEntry::new(
                IsLeading::Unknown,
                SampleDependsOn::DependsOnOthers,
                SampleIsDependedOn::Unknown,
                SampleHasRedundancy::Unknown,
            ),
            PaddingBitsEntry::default(),
            true,
            DegradationPriorityEntry::default(),
        );
        let movie = fragmented_movie(TrackExtendsBox::new(1, 1, 0, 0, SampleFlags::ZERO));
        let samples = [
            Sample::new(
                SampleProperties {
                    track_id: 1,
                    decode_time: 0,
                    sample_duration: 3_000,
                    sample_composition_time_offset: 0,
                    sample_flags: independent,
                    sample_description_index: 1,
                },
                b"AAAAAAAA".to_vec(),
            ),
            Sample::new(
                SampleProperties {
                    track_id: 1,
                    decode_time: 3_000,
                    sample_duration: 3_000,
                    sample_composition_time_offset: 8,
                    sample_flags: dependent,
                    sample_description_index: 1,
                },
                b"BBBB".to_vec(),
            ),
            Sample::new(
                SampleProperties {
                    track_id: 1,
                    decode_time: 6_000,
                    sample_duration: 1_500,
                    sample_composition_time_offset: -8,
                    sample_flags: dependent,
                    sample_description_index: 1,
                },
                b"CC".to_vec(),
            ),
        ];
        let mut writer = MovieFragmentWriter::new(&movie).unwrap();

        writer.begin_fragment(1).unwrap();
        for sample in &samples {
            writer.handle_sample(sample.clone()).unwrap();
        }
        writer.finish_fragment().unwrap();
        let (movie_fragment, media_data) = writer.poll_fragment().unwrap();

        let data_start = MOOF_START + movie_fragment.encoded_len() + MEDIA_DATA_HEADER_LEN;
        let mut decode_times = TrackDecodeTimes::new(&movie).unwrap();
        let extents: Vec<SampleExtent> = sample_extents(
            &movie_fragment,
            &movie,
            MOOF_START,
            &mut decode_times,
            u64::MAX,
        )
        .unwrap()
        .map(Result::unwrap)
        .collect();

        assert_eq!(media_data, b"AAAAAAAABBBBCC");
        assert_eq!(decode_times.decode_time(1), Some(7_500));
        assert_eq!(
            extents,
            [
                SampleExtent::new(
                    SampleProperties {
                        track_id: 1,
                        decode_time: 0,
                        sample_duration: 3_000,
                        sample_composition_time_offset: 0,
                        sample_flags: independent,
                        sample_description_index: 1
                    },
                    1,
                    data_start..data_start + 8
                ),
                SampleExtent::new(
                    SampleProperties {
                        track_id: 1,
                        decode_time: 3_000,
                        sample_duration: 3_000,
                        sample_composition_time_offset: 8,
                        sample_flags: dependent,
                        sample_description_index: 1
                    },
                    1,
                    data_start + 8..data_start + 12
                ),
                SampleExtent::new(
                    SampleProperties {
                        track_id: 1,
                        decode_time: 6_000,
                        sample_duration: 1_500,
                        sample_composition_time_offset: -8,
                        sample_flags: dependent,
                        sample_description_index: 1
                    },
                    1,
                    data_start + 12..data_start + 14
                ),
            ]
        );
    }
}
