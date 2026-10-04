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
        ChunkOffsets, MovieBox, SampleSizeBox, SampleSizes, SampleToChunkBox, TimeToSampleBox,
        TimeToSampleEntry,
    };
    use isobmff_structure::{Error, MediaSegmentDemuxFsm, WantedInput};
    use isobmff_test_support::{
        hybrid_file, indexed_segment_file, presentation_movie, sample_table, segment_file_samples,
        segment_file_with_samples, self_contained_data_reference, track_laid_out,
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
        let file = hybrid_file(true, false);
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
}
