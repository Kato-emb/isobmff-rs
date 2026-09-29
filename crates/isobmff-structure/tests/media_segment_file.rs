//! The samples of a media segment laid out by hand, read back through its structure against the movie it continues

// Why not inside `mod tests`: an inline `mod` adds its own name as a directory
// segment, so a nested one looks for `tests/tests/helpers/media_segment_reading.rs`.
// The `cfg` is what keeps `allow-unwrap-in-tests` reaching the helper from out here.
#[cfg(test)]
#[path = "helpers/media_segment_reading.rs"]
mod reading;

#[cfg(test)]
mod tests {
    use isobmff_structure::MediaSegmentDemuxFsm;
    use isobmff_test_support::{
        indexed_segment_file, presentation_movie, segment_file_samples, segment_file_with_samples,
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
        let mut demux_fsm = MediaSegmentDemuxFsm::new(presentation_movie());

        demux_fsm.handle_input(&segment.bytes).unwrap();
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
        let mut demux_fsm = MediaSegmentDemuxFsm::new(presentation_movie());
        demux_fsm.handle_input(&segment.bytes).unwrap();
        demux_fsm.finish().unwrap();
        drained(&mut demux_fsm);

        demux_fsm.resume_at(second).unwrap();
        demux_fsm
            .handle_input(
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
    fn the_input_stands_after_the_bytes_handed_over_since_the_reading_last_started() {
        let segment = indexed_segment_file();
        let second = *segment.moof_offsets.get(1).unwrap();
        let mut demux_fsm = MediaSegmentDemuxFsm::new(presentation_movie());

        let created = demux_fsm.input_offset();
        demux_fsm.handle_input(&segment.bytes).unwrap();
        let handed = demux_fsm.input_offset();
        demux_fsm.resume_at(second).unwrap();
        let resumed = demux_fsm.input_offset();
        demux_fsm
            .handle_input(
                segment
                    .bytes
                    .get(usize::try_from(second).unwrap()..)
                    .unwrap(),
            )
            .unwrap();

        let segment_len = u64::try_from(segment.bytes.len()).unwrap();
        assert_eq!(
            [created, handed, resumed, demux_fsm.input_offset()],
            [0, segment_len, second, segment_len]
        );
    }
}
