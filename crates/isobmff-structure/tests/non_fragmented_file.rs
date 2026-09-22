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
    use isobmff_structure::NonFragmentedReader;
    use isobmff_test_support::{SAMPLE_CHUNKS, non_fragmented_file, non_fragmented_file_samples};

    #[test]
    fn a_movie_before_its_media_data_has_every_sample_read_as_the_file_arrives() {
        let file = non_fragmented_file(&SAMPLE_CHUNKS, true);
        let mut reader = NonFragmentedReader::new();

        let samples = handed_over_in_order(&mut reader, &file, file.len());

        assert_eq!(samples, non_fragmented_file_samples());
        assert_eq!(reader.wanted_extent(), None);
        assert_eq!(reader.finish(), Ok(()));
    }

    #[test]
    fn a_movie_after_its_media_data_completes_no_sample_and_names_the_bytes_it_lacks() {
        let file = non_fragmented_file(&SAMPLE_CHUNKS, false);
        let mut reader = NonFragmentedReader::new();

        let samples = handed_over_in_order(&mut reader, &file, file.len());

        assert_eq!(samples, []);
        assert_eq!(
            reader.wanted_extent().map(|wanted| fetched(&file, &wanted)),
            Some(b"SAMPLE_1".as_slice())
        );
    }

    #[test]
    fn the_bytes_the_reader_wants_fetched_in_turn_complete_every_sample_however_the_file_was_cut() {
        for file in [
            non_fragmented_file(&SAMPLE_CHUNKS, true),
            non_fragmented_file(&SAMPLE_CHUNKS, false),
        ] {
            for cut_length in [1, 3, 7, 64, file.len().saturating_sub(1), file.len()] {
                assert_eq!(samples_of(&file, cut_length), non_fragmented_file_samples());
            }
        }
    }
}
