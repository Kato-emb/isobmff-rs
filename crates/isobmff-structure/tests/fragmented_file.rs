//! The samples of a fragmented file laid out by hand, read back through its structure

// Why not inside `mod tests`: an inline `mod` adds its own name as a directory
// segment, so a nested one looks for `tests/tests/helpers/reading.rs`. The
// `cfg` is what keeps `allow-unwrap-in-tests` reaching the helper from out here.
#[cfg(test)]
#[path = "helpers/reading.rs"]
mod reading;

#[cfg(test)]
mod tests {
    use super::reading::samples_of;
    use isobmff_test_support::{fragmented_file_samples, fragmented_file_with_samples};

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
}
