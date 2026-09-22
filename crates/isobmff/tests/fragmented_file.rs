//! The samples of a fragmented file laid out by hand, read off a source that seeks

// Why not `cfg(all(test, feature = "std"))` on the module: the
// `tests_outside_test_module` lint reads the module attribute literally and
// fires on anything but a bare `cfg(test)`.
#![cfg(feature = "std")]

#[cfg(test)]
mod tests {
    use std::io;

    use isobmff::{FragmentedDemuxer, Sample};
    use isobmff_test_support::{fragmented_file_samples, fragmented_file_with_samples};

    #[test]
    fn a_source_that_seeks_has_every_sample_read_off_it() {
        let read_back: Vec<Sample> =
            FragmentedDemuxer::new(io::Cursor::new(fragmented_file_with_samples()))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();

        assert_eq!(read_back, fragmented_file_samples());
    }
}
