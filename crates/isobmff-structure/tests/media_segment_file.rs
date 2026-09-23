//! The samples of a media segment laid out by hand, read back through its structure against the movie it continues

// Why not inside `mod tests`: an inline `mod` adds its own name as a directory
// segment, so a nested one looks for `tests/tests/helpers/media_segment_reading.rs`.
// The `cfg` is what keeps `allow-unwrap-in-tests` reaching the helper from out here.
#[cfg(test)]
#[path = "helpers/media_segment_reading.rs"]
mod reading;

#[cfg(test)]
mod tests {
    use isobmff_sample::Sample;
    use isobmff_structure::MediaSegmentReader;
    use isobmff_test_support::{
        indexed_segment_file, presentation_movie, segment_file_samples, segment_file_with_samples,
    };

    use super::reading::samples_of;

    /// Takes every sample the reader has completed
    fn drained(reader: &mut MediaSegmentReader) -> Vec<Sample> {
        let mut samples = Vec::new();
        while let Some(sample) = reader.poll_sample() {
            samples.push(sample);
        }

        samples
    }

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
        let mut reader = MediaSegmentReader::new(presentation_movie());

        reader.handle_input(&segment.bytes).unwrap();
        reader.finish().unwrap();

        assert_eq!(drained(&mut reader), segment.fragment_samples.concat());
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
            [segment.moof_offsets.as_slice()]
        );
    }

    #[test]
    fn resuming_at_the_second_fragment_yields_its_samples_alone() {
        let segment = indexed_segment_file();
        let second = *segment.moof_offsets.get(1).unwrap();
        let mut reader = MediaSegmentReader::new(presentation_movie());
        reader.handle_input(&segment.bytes).unwrap();
        reader.finish().unwrap();
        drained(&mut reader);

        reader.resume_at(second).unwrap();
        reader
            .handle_input(
                segment
                    .bytes
                    .get(usize::try_from(second).unwrap()..)
                    .unwrap(),
            )
            .unwrap();
        reader.finish().unwrap();

        assert_eq!(
            &drained(&mut reader),
            segment.fragment_samples.get(1).unwrap()
        );
    }
}
