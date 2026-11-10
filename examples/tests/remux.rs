//! Files the muxers write of what the demuxer read, read back the same

#[cfg(test)]
mod tests {
    use isobmff_examples::demux::read;
    use isobmff_examples::remux::{fragmented_movie_file, movie_file};

    #[test]
    fn a_file_written_again_as_a_movie_file_or_in_fragments_holds_the_same_samples() {
        for file in [
            include_bytes!("../../crates/isobmff/tests/fixtures/movie_moov_last.mp4").as_slice(),
            include_bytes!("../../crates/isobmff/tests/fixtures/fragmented.mp4"),
        ] {
            let source = read(file).unwrap();
            for written in [movie_file(&source), fragmented_movie_file(&source)] {
                let read_back = read(&written.unwrap()).unwrap();
                assert_eq!(
                    (read_back.samples, read_back.errors),
                    (source.samples.clone(), Vec::new())
                );
            }
        }
    }
}
