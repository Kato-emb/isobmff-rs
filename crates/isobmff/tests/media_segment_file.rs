//! The samples of a media segment laid out by hand, read off a source that seeks against the movie it continues

// Why not `cfg(all(test, feature = "std"))` on the module: the
// `tests_outside_test_module` lint reads the module attribute literally and
// fires on anything but a bare `cfg(test)`.
#![cfg(feature = "std")]

#[cfg(test)]
mod tests {
    use std::io;

    use isobmff::{MediaSegmentDemuxer, Sample};
    use isobmff_test_support::{
        presentation_movie, segment_file_samples, segment_file_with_samples,
    };

    #[test]
    fn a_source_that_seeks_has_every_sample_read_off_it() {
        let read_back: Vec<Sample> = MediaSegmentDemuxer::new(
            io::Cursor::new(segment_file_with_samples()),
            presentation_movie(),
        )
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

        assert_eq!(read_back, segment_file_samples());
    }
}
