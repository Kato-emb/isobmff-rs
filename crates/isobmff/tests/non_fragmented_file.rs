//! The samples of a non-fragmented file, read back through its structure with the movie before and after the media data

#[cfg(test)]
mod tests {
    use core::ops::Range;

    use isobmff::{NonFragmentedReader, Sample};
    use isobmff_test_support::{SAMPLE_DURATION, non_fragmented_file};

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

    /// The bytes of `file` lying at `extent`
    fn fetched<'file>(file: &'file [u8], extent: &Range<u64>) -> &'file [u8] {
        file.get(usize::try_from(extent.start).unwrap()..usize::try_from(extent.end).unwrap())
            .unwrap()
    }

    /// Hands `file` over in order, `cut_length` bytes at a time, and returns the samples that completed
    fn handed_over_in_order(
        reader: &mut NonFragmentedReader,
        file: &[u8],
        cut_length: usize,
    ) -> Vec<Sample> {
        let mut samples = Vec::new();
        let mut offset = 0;

        for arriving in file.chunks(cut_length) {
            reader.handle_input(offset, arriving).unwrap();
            offset = offset.saturating_add(u64::try_from(arriving.len()).unwrap());
            while let Some(sample) = reader.poll_sample() {
                samples.push(sample);
            }
        }

        samples
    }

    /// The samples `file` carries, read off it `cut_length` bytes at a time and then off the bytes it wants fetched
    fn samples_of(file: &[u8], cut_length: usize) -> Vec<Sample> {
        let mut reader = NonFragmentedReader::new();
        let mut samples = handed_over_in_order(&mut reader, file, cut_length);

        while let Some(wanted) = reader.wanted_extent() {
            reader
                .handle_data(wanted.start, fetched(file, &wanted))
                .unwrap();
            while let Some(sample) = reader.poll_sample() {
                samples.push(sample);
            }
        }
        reader.finish().unwrap();
        while let Some(sample) = reader.poll_sample() {
            samples.push(sample);
        }

        samples
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
}
