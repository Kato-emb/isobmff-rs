//! The samples a writer laid down as a non-fragmented movie file, read back off that file

// Why not inside `mod tests`: an inline `mod` adds its own name as a directory
// segment, so a nested one looks for `tests/tests/helpers/…`. The `cfg` is
// what keeps `allow-unwrap-in-tests` reaching the helper from out here.
#[cfg(test)]
#[path = "helpers/non_fragmented_reading.rs"]
mod reading;

#[cfg(test)]
mod tests {
    use super::reading::samples_of;
    use isobmff::{
        BoxEvent, BoxType, MovieBox, MovieHeaderBox, Mp4EpochSeconds, NonFragmentedWriter, Sample,
    };
    use isobmff_test_support::{events_of, file_type, track};

    /// Ticks a second the media of the movie is timed in
    const TIMESCALE: u32 = 90_000;

    /// Movie of two tracks declaring no sample yet, the template the writer fills in
    fn movie() -> MovieBox {
        let epoch = Mp4EpochSeconds::from_seconds(0);

        MovieBox::new(
            MovieHeaderBox::new(epoch, epoch, TIMESCALE, 0, 3),
            vec![track(1), track(2)],
            None,
        )
        .unwrap()
    }

    /// The samples the two tracks carry, chunk by chunk, the tracks taking turns
    fn declared_chunks() -> Vec<Vec<Sample>> {
        let video =
            |decode_time, data: &[u8]| Sample::new(1, decode_time, 3_000, 0, 0, 1, data.to_vec());
        let audio =
            |decode_time, data: &[u8]| Sample::new(2, decode_time, 1_024, 0, 0, 1, data.to_vec());

        vec![
            vec![video(0, b"VIDEO_01"), video(3_000, b"VIDEO_02")],
            vec![
                audio(0, b"AUD1"),
                audio(1_024, b"AUD2"),
                audio(2_048, b"AUD3"),
            ],
            vec![video(6_000, b"VIDEO_03")],
            vec![audio(3_072, b"AUD4")],
        ]
    }

    /// The file the chunks make: the brands, one `mdat` per chunk, then the movie
    fn written_file(chunks: Vec<Vec<Sample>>) -> Vec<u8> {
        let mut writer = NonFragmentedWriter::new();
        let mut file = Vec::new();

        writer.handle_file_type(file_type()).unwrap();
        writer.handle_movie(movie()).unwrap();
        for chunk in chunks {
            writer.begin_chunk().unwrap();
            for sample in chunk {
                writer.handle_sample(sample).unwrap();
            }
        }
        writer.finish().unwrap();

        while let Some(written) = writer.poll_output() {
            file.extend_from_slice(&written);
        }

        file
    }

    #[test]
    fn the_samples_are_read_back_as_they_were_handed_over_chunk_by_chunk() {
        let file = written_file(declared_chunks());

        assert_eq!(samples_of(&file, file.len()), declared_chunks().concat());
    }

    #[test]
    fn each_chunk_is_laid_down_as_its_own_media_data_box_and_the_movie_comes_last() {
        let file = written_file(declared_chunks());

        let top_level: Vec<BoxType> = events_of(&file, file.len())
            .unwrap()
            .into_iter()
            .filter_map(|(_extent, event)| {
                if let BoxEvent::Header(header) = event {
                    Some(header.box_type())
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(
            top_level,
            [b"ftyp", b"mdat", b"mdat", b"mdat", b"mdat", b"moov"]
                .map(|fourcc| BoxType::compact(*fourcc))
        );
    }
}
