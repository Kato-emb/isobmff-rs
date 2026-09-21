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
    use isobmff::{NonFragmentedReader, Sample};
    use isobmff_test_support::{SAMPLE_DURATION, non_fragmented_file};
    #[cfg(feature = "std")]
    use {isobmff::NonFragmentedDemuxer, std::io};

    /// The samples of the synthetic file, chunk by chunk
    const CHUNKS: [&[&[u8]]; 3] = [
        &[b"SAMPLE_1", b"SAMPLE_2"],
        &[b"SAMPLE_3"],
        &[b"SAMPLE_4", b"SAMPLE_5", b"SAMPLE_6"],
    ];

    /// The samples the synthetic file was built to carry
    fn declared_samples() -> Vec<Sample> {
        let mut decode_time = 0;

        CHUNKS
            .iter()
            .flat_map(|chunk| chunk.iter())
            .map(|data| {
                let sample = Sample::new(1, decode_time, SAMPLE_DURATION, 0, 0, 1, data.to_vec());
                decode_time = decode_time.saturating_add(u64::from(SAMPLE_DURATION));

                sample
            })
            .collect()
    }

    #[test]
    fn a_movie_before_its_media_data_has_every_sample_read_as_the_file_arrives() {
        let file = non_fragmented_file(&CHUNKS, true);
        let mut reader = NonFragmentedReader::new();

        let samples = handed_over_in_order(&mut reader, &file, file.len());

        assert_eq!(samples, declared_samples());
        assert_eq!(reader.wanted_extent(), None);
        assert_eq!(reader.finish(), Ok(()));
    }

    #[test]
    fn a_movie_after_its_media_data_completes_no_sample_and_names_the_bytes_it_lacks() {
        let file = non_fragmented_file(&CHUNKS, false);
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
            non_fragmented_file(&CHUNKS, true),
            non_fragmented_file(&CHUNKS, false),
        ] {
            for cut_length in [1, 3, 7, 64, file.len().saturating_sub(1), file.len()] {
                assert_eq!(samples_of(&file, cut_length), declared_samples());
            }
        }
    }

    #[cfg(feature = "std")]
    #[test]
    fn a_source_that_seeks_has_every_sample_read_off_it_wherever_the_movie_lies() {
        for movie_first in [true, false] {
            let file = non_fragmented_file(&CHUNKS, movie_first);

            let read_back: Vec<Sample> = NonFragmentedDemuxer::new(io::Cursor::new(file))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();

            assert_eq!(read_back, declared_samples());
        }
    }
}
