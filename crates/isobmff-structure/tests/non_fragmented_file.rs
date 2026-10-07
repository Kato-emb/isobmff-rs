//! The samples of a non-fragmented file, read back through its structure with the movie before and after the media data

// Why not inside `mod tests`: an inline `mod` adds its own name as a directory
// segment, so a nested one looks for `tests/tests/helpers/non_fragmented_reading.rs`.
// The `cfg` is what keeps `allow-unwrap-in-tests` reaching the helper from out here.
#[cfg(test)]
#[path = "helpers/non_fragmented_reading.rs"]
mod reading;

#[cfg(test)]
mod tests {
    use isobmff_sample::Sample;

    use super::reading::{drained, samples_of};
    use isobmff_structure::{Error, MovieDemuxFsm, WantedInput};
    use isobmff_test_support::{
        SAMPLE_CHUNKS, movie_fragment, non_fragmented_file, non_fragmented_file_samples, written,
    };

    /// The bytes of `file` a read of known length wants
    fn fetched(file: &[u8], wanted: WantedInput) -> &[u8] {
        let start = usize::try_from(wanted.offset()).unwrap();
        let length = usize::try_from(wanted.length().unwrap()).unwrap();

        file.get(start..)
            .and_then(|rest| rest.get(..length))
            .unwrap()
    }

    /// Hands `file` over in order, `cut_length` bytes at a time, and returns the samples that completed
    fn handed_over_in_order(
        demux_fsm: &mut MovieDemuxFsm,
        file: &[u8],
        cut_length: usize,
    ) -> Vec<Sample> {
        let mut samples = Vec::new();

        for (offset, arriving) in (0..).step_by(cut_length).zip(file.chunks(cut_length)) {
            demux_fsm.handle_input(offset, arriving).unwrap();
            samples.extend(drained(demux_fsm));
        }

        samples
    }

    #[test]
    fn a_movie_before_its_media_data_has_every_sample_read_as_the_file_arrives() {
        let file = non_fragmented_file(&SAMPLE_CHUNKS, true);
        let mut demux_fsm = MovieDemuxFsm::new();

        let samples = handed_over_in_order(&mut demux_fsm, &file, file.len());

        assert_eq!(samples, non_fragmented_file_samples());
        assert_eq!(demux_fsm.wanted_input().and_then(WantedInput::length), None);
        assert_eq!(demux_fsm.finish(), Ok(()));
    }

    #[test]
    fn a_movie_after_its_media_data_completes_no_sample_and_names_the_bytes_it_lacks() {
        let file = non_fragmented_file(&SAMPLE_CHUNKS, false);
        let mut demux_fsm = MovieDemuxFsm::new();

        let samples = handed_over_in_order(&mut demux_fsm, &file, file.len());

        assert_eq!(samples, []);
        assert_eq!(
            demux_fsm
                .wanted_input()
                .map(|wanted| fetched(&file, wanted)),
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
    fn the_continuation_is_wanted_after_the_bytes_handed_over_so_far() {
        let file = non_fragmented_file(&SAMPLE_CHUNKS, true);
        let mut demux_fsm = MovieDemuxFsm::new();

        let created = demux_fsm.wanted_input();
        demux_fsm.handle_input(0, &file).unwrap();

        assert_eq!(
            [created, demux_fsm.wanted_input()],
            [
                Some(WantedInput::new(0, None)),
                Some(WantedInput::new(u64::try_from(file.len()).unwrap(), None))
            ]
        );
    }

    #[test]
    fn a_movie_before_its_media_data_names_nothing_wanted_however_the_file_is_cut() {
        let file = non_fragmented_file(&SAMPLE_CHUNKS, true);

        for cut_length in [1, 3, 7, 64, file.len()] {
            let mut demux_fsm = MovieDemuxFsm::new();
            let mut wanted = Vec::new();
            for (offset, arriving) in (0..).step_by(cut_length).zip(file.chunks(cut_length)) {
                demux_fsm.handle_input(offset, arriving).unwrap();
                wanted.extend(demux_fsm.wanted_input().and_then(WantedInput::length));
            }

            assert_eq!(wanted, []);
        }
    }

    #[test]
    fn a_movie_after_its_media_data_names_each_sample_it_lacks_once_in_turn() {
        let file = non_fragmented_file(&SAMPLE_CHUNKS, false);
        let mut demux_fsm = MovieDemuxFsm::new();
        handed_over_in_order(&mut demux_fsm, &file, 7);

        let mut wanted = Vec::new();
        while let Some(extent) = demux_fsm
            .wanted_input()
            .filter(|wanted| wanted.length().is_some())
        {
            let bytes = fetched(&file, extent);
            demux_fsm.handle_input(extent.offset(), bytes).unwrap();
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

    #[test]
    fn a_fragment_after_a_movie_carrying_no_mvex_is_rejected_for_the_missing_movie_extends() {
        let file = [
            non_fragmented_file(&SAMPLE_CHUNKS, true),
            written(&movie_fragment()),
        ]
        .concat();
        let mut demux_fsm = MovieDemuxFsm::new();

        assert!(matches!(
            demux_fsm.handle_input(0, &file),
            Err(Error::Sample {
                error: isobmff_sample::Error::MissingMovieExtends { .. },
                ..
            })
        ));
    }
}
