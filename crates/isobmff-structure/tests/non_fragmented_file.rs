//! The samples of a non-fragmented file, read back through its structure with the movie before and after the media data

// Why not inside `mod tests`: an inline `mod` adds its own name as a directory
// segment, so a nested one looks for `tests/tests/helpers/non_fragmented_reading.rs`.
// The `cfg` is what keeps `allow-unwrap-in-tests` reaching the helper from out here.
#[cfg(test)]
#[path = "helpers/non_fragmented_reading.rs"]
mod reading;

#[cfg(test)]
mod tests {
    use super::reading::{fetched, handed_over_in_order, samples_of};
    use isobmff_structure::NonFragmentedDemuxFsm;
    use isobmff_test_support::{SAMPLE_CHUNKS, non_fragmented_file, non_fragmented_file_samples};

    #[test]
    fn a_movie_before_its_media_data_has_every_sample_read_as_the_file_arrives() {
        let file = non_fragmented_file(&SAMPLE_CHUNKS, true);
        let mut demux_fsm = NonFragmentedDemuxFsm::new();

        let samples = handed_over_in_order(&mut demux_fsm, &file, file.len());

        assert_eq!(samples, non_fragmented_file_samples());
        assert_eq!(demux_fsm.wanted_extent(), None);
        assert_eq!(demux_fsm.finish(), Ok(()));
    }

    #[test]
    fn a_movie_after_its_media_data_completes_no_sample_and_names_the_bytes_it_lacks() {
        let file = non_fragmented_file(&SAMPLE_CHUNKS, false);
        let mut demux_fsm = NonFragmentedDemuxFsm::new();

        let samples = handed_over_in_order(&mut demux_fsm, &file, file.len());

        assert_eq!(samples, []);
        assert_eq!(
            demux_fsm
                .wanted_extent()
                .map(|wanted| fetched(&file, &wanted)),
            Some(b"SAMPLE_1".as_slice())
        );
    }

    #[test]
    fn the_bytes_the_demux_fsm_wants_fetched_in_turn_complete_every_sample_however_the_file_was_cut()
     {
        for file in [
            non_fragmented_file(&SAMPLE_CHUNKS, true),
            non_fragmented_file(&SAMPLE_CHUNKS, false),
        ] {
            for cut_length in [1, 3, 7, 64, file.len().saturating_sub(1), file.len()] {
                assert_eq!(samples_of(&file, cut_length), non_fragmented_file_samples());
            }
        }
    }

    #[test]
    fn the_input_stands_after_the_bytes_handed_over_so_far() {
        let file = non_fragmented_file(&SAMPLE_CHUNKS, true);
        let mut demux_fsm = NonFragmentedDemuxFsm::new();

        let created = demux_fsm.input_offset();
        demux_fsm.handle_input(&file).unwrap();

        assert_eq!(
            [created, demux_fsm.input_offset()],
            [0, u64::try_from(file.len()).unwrap()]
        );
    }

    #[test]
    fn a_movie_before_its_media_data_names_nothing_wanted_however_the_file_is_cut() {
        let file = non_fragmented_file(&SAMPLE_CHUNKS, true);

        for cut_length in [1, 3, 7, 64, file.len()] {
            let mut demux_fsm = NonFragmentedDemuxFsm::new();
            let mut wanted = Vec::new();
            for arriving in file.chunks(cut_length) {
                demux_fsm.handle_input(arriving).unwrap();
                wanted.extend(demux_fsm.wanted_extent());
            }

            assert_eq!(wanted, []);
        }
    }

    #[test]
    fn a_movie_after_its_media_data_names_each_sample_it_lacks_once_in_turn() {
        let file = non_fragmented_file(&SAMPLE_CHUNKS, false);
        let mut demux_fsm = NonFragmentedDemuxFsm::new();
        handed_over_in_order(&mut demux_fsm, &file, 7);

        let mut wanted = Vec::new();
        while let Some(extent) = demux_fsm.wanted_extent() {
            let bytes = fetched(&file, &extent);
            demux_fsm.handle_data(extent.start, bytes).unwrap();
            wanted.push(bytes);
        }

        assert_eq!(
            wanted,
            SAMPLE_CHUNKS
                .iter()
                .flat_map(|chunk| chunk.iter().copied())
                .collect::<Vec<_>>()
        );
    }
}
