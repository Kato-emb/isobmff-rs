//! The samples of a fragmented file laid out by hand, read back through its structure

// Why not inside `mod tests`: an inline `mod` adds its own name as a directory
// segment, so a nested one looks for `tests/tests/helpers/reading.rs`. The
// `cfg` is what keeps `allow-unwrap-in-tests` reaching the helper from out here.
#[cfg(test)]
#[path = "helpers/reading.rs"]
mod reading;

#[cfg(test)]
mod tests {
    use isobmff_core::BoxType;
    use isobmff_sample::Sample;
    use isobmff_structure::{Error, ErrorKind, FragmentedReader};
    use isobmff_test_support::{
        IndexedFile, fragmented_file_samples, fragmented_file_with_samples,
        indexed_fragmented_file, indexed_fragmented_file_without_decode_times,
    };

    use super::reading::samples_of;

    /// Takes every sample the reader has completed
    fn drained(reader: &mut FragmentedReader) -> Vec<Sample> {
        let mut samples = Vec::new();
        while let Some(sample) = reader.poll_sample() {
            samples.push(sample);
        }

        samples
    }

    /// Reader that read `file` whole and was declared over
    fn read_whole(file: &IndexedFile) -> FragmentedReader {
        let mut reader = FragmentedReader::new();
        reader.handle_input(&file.bytes).unwrap();
        reader.finish().unwrap();

        reader
    }

    /// Resumes `reader` at `offset` and hands it the rest of `file` from there
    fn resumed_at(
        reader: &mut FragmentedReader,
        file: &IndexedFile,
        offset: u64,
    ) -> Result<(), Error> {
        reader.resume_at(offset)?;
        reader.handle_input(file.bytes.get(usize::try_from(offset).unwrap()..).unwrap())?;
        reader.finish()
    }

    #[test]
    fn the_samples_of_a_fragmented_file_are_read_off_the_bytes_it_lies_as() {
        let file = fragmented_file_with_samples();

        assert_eq!(samples_of(&file, file.len()), fragmented_file_samples());
    }

    #[test]
    fn the_samples_are_the_same_however_the_file_was_cut() {
        let file = fragmented_file_with_samples();

        for cut_length in [1, 3, 7, 64, file.len().saturating_sub(1)] {
            assert_eq!(samples_of(&file, cut_length), fragmented_file_samples());
        }
    }

    #[test]
    fn a_file_read_in_order_yields_every_sample_and_both_indexes_pointing_at_its_fragments() {
        let file = indexed_fragmented_file();

        let mut reader = read_whole(&file);

        assert_eq!(drained(&mut reader), file.fragment_samples.concat());
        assert_eq!(
            reader
                .segment_indexes()
                .iter()
                .map(|segment_index| segment_index
                    .subsegments()
                    .iter()
                    .map(|subsegment| subsegment.extent().start)
                    .collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            [file.moof_offsets.as_slice()]
        );
        assert_eq!(
            reader
                .movie_fragment_random_access()
                .unwrap()
                .tfra()
                .iter()
                .flat_map(|tfra| tfra.entries())
                .map(|entry| entry.moof_offset())
                .collect::<Vec<_>>(),
            file.moof_offsets
        );
    }

    #[test]
    fn resuming_at_the_second_fragment_yields_its_samples_alone() {
        let file = indexed_fragmented_file();
        let second = *file.moof_offsets.get(1).unwrap();

        let mut finished = read_whole(&file);
        drained(&mut finished);
        resumed_at(&mut finished, &file, second).unwrap();

        let mut reading = FragmentedReader::new();
        reading
            .handle_input(file.bytes.get(..usize::try_from(second).unwrap()).unwrap())
            .unwrap();
        drained(&mut reading);
        resumed_at(&mut reading, &file, second).unwrap();

        let second_samples = file.fragment_samples.get(1).unwrap();
        assert_eq!(&drained(&mut finished), second_samples);
        assert_eq!(&drained(&mut reading), second_samples);
    }

    #[test]
    fn resuming_at_media_data_is_out_of_order_and_leaves_the_reader_failed() {
        let file = indexed_fragmented_file();
        let second = *file.moof_offsets.get(1).unwrap();
        let moof_size = file
            .bytes
            .get(usize::try_from(second).unwrap()..)
            .and_then(|moof| moof.first_chunk::<4>())
            .unwrap();
        let media_data = second.saturating_add(u64::from(u32::from_be_bytes(*moof_size)));
        let out_of_order = Error::box_out_of_order(BoxType::compact(*b"mdat"));

        let mut reader = read_whole(&file);

        assert_eq!(
            resumed_at(&mut reader, &file, media_data),
            Err(out_of_order)
        );
        assert_eq!(reader.resume_at(second), Err(out_of_order));
    }

    #[test]
    fn resuming_at_a_fragment_stating_no_decode_time_is_refused() {
        let file = indexed_fragmented_file_without_decode_times();

        let mut reader = read_whole(&file);

        assert_eq!(
            resumed_at(&mut reader, &file, *file.moof_offsets.get(1).unwrap()).map_err(Error::kind),
            Err(ErrorKind::Sample(
                isobmff_sample::ErrorKind::MissingDecodeTime
            ))
        );
    }
}
