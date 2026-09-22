//! The samples of a media segment laid out by hand, read back through its structure against the movie it continues

// Why not inside `mod tests`: an inline `mod` adds its own name as a directory
// segment, so a nested one looks for `tests/tests/helpers/media_segment_reading.rs`.
// The `cfg` is what keeps `allow-unwrap-in-tests` reaching the helper from out here.
#[cfg(test)]
#[path = "helpers/media_segment_reading.rs"]
mod reading;

#[cfg(test)]
mod tests {
    use super::reading::samples_of;
    use isobmff_test_support::{
        presentation_movie, segment_file_samples, segment_file_with_samples,
    };

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
}
