//! The samples of a non-fragmented file, read off a source that seeks with the movie before and after the media data

// Why not `cfg(all(test, feature = "std"))` on the module: the
// `tests_outside_test_module` lint reads the module attribute literally and
// fires on anything but a bare `cfg(test)`.
#![cfg(feature = "std")]

#[cfg(test)]
mod tests {
    use std::io;

    use isobmff::{NonFragmentedDemuxer, Sample};
    use isobmff_test_support::{SAMPLE_CHUNKS, non_fragmented_file, non_fragmented_file_samples};

    #[test]
    fn a_source_that_seeks_has_every_sample_read_off_it_wherever_the_movie_lies() {
        for movie_first in [true, false] {
            let file = non_fragmented_file(&SAMPLE_CHUNKS, movie_first);

            let read_back: Vec<Sample> = NonFragmentedDemuxer::new(io::Cursor::new(file))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();

            assert_eq!(read_back, non_fragmented_file_samples());
        }
    }
}
