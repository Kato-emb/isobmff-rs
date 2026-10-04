//! The samples of a media segment laid out by hand, read back through its structure against the movie it continues

// Why not inside `mod tests`: an inline `mod` adds its own name as a directory
// segment, so a nested one looks for `tests/tests/helpers/media_segment_reading.rs`.
// The `cfg` is what keeps `allow-unwrap-in-tests` reaching the helper from out here.
#[cfg(test)]
#[path = "helpers/media_segment_reading.rs"]
mod reading;

#[cfg(test)]
mod tests {
    use isobmff_boxes::{
        ChunkOffsets, MediaDataBox, MovieBox, MovieFragmentBox, MovieFragmentHeaderBox,
        SampleSizeBox, SampleSizes, SampleToChunkBox, SegmentTypeBox, TimeToSampleBox,
        TimeToSampleEntry, TrackFragmentBox, TrackFragmentHeaderBox, TrackFragmentHeaderFlags,
        TrackRunBox, TrackRunSample,
    };
    use isobmff_core::{BoxEncode, FourCC};
    use isobmff_structure::{Error, MediaSegmentDemuxFsm, WantedInput};
    use isobmff_test_support::{
        SAMPLE_CHUNKS, fragmented_file_with_movie_samples, indexed_segment_file,
        indexed_segment_file_without_decode_times, non_fragmented_file_samples, presentation_movie,
        sample_table, segment_file_samples, segment_file_with_samples, segment_type,
        self_contained_data_reference, track_laid_out, written,
    };

    use super::reading::{drained, samples_of};

    #[test]
    fn the_samples_of_a_media_segment_are_read_off_the_bytes_it_lies_as() {
        let segment = segment_file_with_samples();

        assert_eq!(
            samples_of(presentation_movie(), &segment, segment.len()),
            segment_file_samples()
        );
    }

    #[test]
    fn the_samples_are_the_same_however_the_segment_was_cut() {
        let segment = segment_file_with_samples();

        for cut_length in [1, 3, 7, 64, segment.len().saturating_sub(1)] {
            assert_eq!(
                samples_of(presentation_movie(), &segment, cut_length),
                segment_file_samples()
            );
        }
    }

    #[test]
    fn a_segment_read_in_order_yields_every_sample_and_an_index_pointing_at_its_fragments() {
        let segment = indexed_segment_file();
        let mut demux_fsm = MediaSegmentDemuxFsm::new(presentation_movie()).unwrap();

        demux_fsm.handle_input(0, &segment.bytes).unwrap();
        demux_fsm.finish().unwrap();

        assert_eq!(drained(&mut demux_fsm), segment.fragment_samples.concat());
        assert_eq!(
            demux_fsm
                .segment_indexes()
                .iter()
                .map(|segment_index| segment_index
                    .subsegments()
                    .iter()
                    .map(|subsegment| subsegment.extent().start)
                    .collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            [segment.moof_offsets.as_slice()]
        );
    }

    #[test]
    fn resuming_at_the_second_fragment_yields_its_samples_alone() {
        let segment = indexed_segment_file();
        let second = *segment.moof_offsets.get(1).unwrap();
        let mut demux_fsm = MediaSegmentDemuxFsm::new(presentation_movie()).unwrap();
        demux_fsm.handle_input(0, &segment.bytes).unwrap();
        demux_fsm.finish().unwrap();
        drained(&mut demux_fsm);

        demux_fsm.resume_at(second).unwrap();
        demux_fsm
            .handle_input(
                second,
                segment
                    .bytes
                    .get(usize::try_from(second).unwrap()..)
                    .unwrap(),
            )
            .unwrap();
        demux_fsm.finish().unwrap();

        assert_eq!(
            &drained(&mut demux_fsm),
            segment.fragment_samples.get(1).unwrap()
        );
    }

    #[test]
    fn the_continuation_is_wanted_after_the_bytes_handed_over_since_the_reading_last_started() {
        let segment = indexed_segment_file();
        let second = *segment.moof_offsets.get(1).unwrap();
        let mut demux_fsm = MediaSegmentDemuxFsm::new(presentation_movie()).unwrap();

        let created = demux_fsm.wanted_input();
        demux_fsm.handle_input(0, &segment.bytes).unwrap();
        let handed = demux_fsm.wanted_input();
        demux_fsm.resume_at(second).unwrap();
        let resumed = demux_fsm.wanted_input();
        demux_fsm
            .handle_input(
                second,
                segment
                    .bytes
                    .get(usize::try_from(second).unwrap()..)
                    .unwrap(),
            )
            .unwrap();

        let segment_length = u64::try_from(segment.bytes.len()).unwrap();
        assert_eq!(
            [created, handed, resumed, demux_fsm.wanted_input()],
            [
                Some(WantedInput::new(0, None)),
                Some(WantedInput::new(segment_length, None)),
                Some(WantedInput::new(second, None)),
                Some(WantedInput::new(segment_length, None))
            ]
        );
    }

    #[test]
    fn a_first_fragment_stating_no_decode_time_starts_where_the_sample_table_of_the_movie_leaves_its_track()
     {
        let file = fragmented_file_with_movie_samples(true, false);
        let segment = file
            .bytes
            .get(usize::try_from(file.moof_offset).unwrap()..)
            .unwrap();

        assert_eq!(samples_of(file.movie, segment, 7), file.fragment_samples);
    }

    #[test]
    fn a_movie_whose_sample_table_runs_past_64_bits_of_decode_time_is_rejected() {
        let longest_run = TimeToSampleEntry::new(u32::MAX, u32::MAX);
        let stbl = sample_table(
            TimeToSampleBox::new(vec![longest_run, longest_run]),
            SampleToChunkBox::new(Vec::new()),
            SampleSizes::Stsz(SampleSizeBox::from_sizes([])),
            ChunkOffsets::from_offsets([]),
        );
        let movie = MovieBox::new_fragmented(
            90_000,
            vec![track_laid_out(1, self_contained_data_reference(), stbl)],
        )
        .unwrap();

        assert_eq!(
            MediaSegmentDemuxFsm::new(movie).map(drop),
            Err(Error::from(isobmff_sample::Error::decode_time_overflow(1)))
        );
    }

    /// Brands of a later segment, other than those [`segment_type`] declares
    fn later_segment_type() -> SegmentTypeBox {
        SegmentTypeBox::new(
            FourCC::new(*b"msix"),
            1,
            vec![FourCC::new(*b"msix"), FourCC::new(*b"dash")],
        )
    }

    #[test]
    fn segments_concatenated_each_with_its_brands_read_as_one_with_times_carried_on() {
        let segment = indexed_segment_file_without_decode_times();
        let (first, second) = segment
            .bytes
            .split_at(usize::try_from(*segment.moof_offsets.get(1).unwrap()).unwrap());
        let concatenated = [first, &written(&later_segment_type()), second].concat();

        for cut_length in [7, concatenated.len()] {
            assert_eq!(
                samples_of(presentation_movie(), &concatenated, cut_length),
                segment.fragment_samples.concat(),
                "cut length: {cut_length}"
            );
        }
    }

    #[test]
    fn the_brands_of_the_segment_read_last_are_the_ones_there_to_read() {
        let first_segment = segment_file_with_samples();
        let mut demux_fsm = MediaSegmentDemuxFsm::new(presentation_movie()).unwrap();

        demux_fsm.handle_input(0, &first_segment).unwrap();
        let after_the_first = demux_fsm.segment_type().cloned();
        demux_fsm
            .handle_input(
                u64::try_from(first_segment.len()).unwrap(),
                &written(&later_segment_type()),
            )
            .unwrap();

        assert_eq!(
            (after_the_first, demux_fsm.segment_type().cloned()),
            (Some(segment_type()), Some(later_segment_type()))
        );
    }

    #[test]
    fn a_fragment_addressing_media_data_before_it_is_read_through_the_bytes_wanted_back() {
        let data = SAMPLE_CHUNKS.first().unwrap();
        let media_data = MediaDataBox::new(data.concat());
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
            vec![
                TrackRunBox::new(
                    Some(-i32::try_from(media_data.payload_len()).unwrap()),
                    None,
                    vec![TrackRunSample::new(None, None, None, None); data.len()],
                )
                .unwrap(),
            ],
        )
        .unwrap();
        let segment = [
            written(&segment_type()),
            written(&media_data),
            written(&MovieFragmentBox::new(
                MovieFragmentHeaderBox::new(1),
                vec![track_fragment],
            )),
        ]
        .concat();

        assert_eq!(
            samples_of(presentation_movie(), &segment, 7),
            non_fragmented_file_samples()
                .into_iter()
                .take(data.len())
                .collect::<Vec<_>>()
        );
    }
}
