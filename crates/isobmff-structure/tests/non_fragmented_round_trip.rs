//! The samples a writer laid down as a non-fragmented movie file, read back off that file

// Why not inside `mod tests`: an inline `mod` adds its own name as a directory
// segment, so a nested one looks for `tests/tests/helpers/non_fragmented_reading.rs`.
// The `cfg` is what keeps `allow-unwrap-in-tests` reaching the helper from out here.
#[cfg(test)]
#[path = "helpers/non_fragmented_reading.rs"]
mod reading;

#[cfg(test)]
mod tests {
    use super::reading::samples_of;
    use isobmff_boxes::{FileTypeBox, HeaderDuration, MovieBox, MovieHeaderBox, SampleFlags};
    use isobmff_core::{BoxType, FourCC, Mp4EpochSeconds};
    use isobmff_sample::Sample;
    use isobmff_sequence::BoxEvent;
    use isobmff_structure::{NonFragmentedReader, NonFragmentedWriter};
    use isobmff_test_support::{events_of, file_type, track};

    /// Ticks a second the media of the movie is timed in
    const TIMESCALE: u32 = 90_000;

    /// Movie of two tracks declaring no sample yet, the template the writer fills in
    fn movie() -> MovieBox {
        let epoch = Mp4EpochSeconds::from_seconds(0);

        MovieBox::new(
            MovieHeaderBox::new(epoch, epoch, TIMESCALE, HeaderDuration::ZERO, 3),
            vec![track(1), track(2)],
            None,
        )
        .unwrap()
    }

    /// The samples the two tracks carry, chunk by chunk, the tracks taking turns
    fn two_track_chunks() -> Vec<Vec<Sample>> {
        let video = |decode_time, data: &[u8]| {
            Sample::new(
                1,
                decode_time,
                3_000,
                0,
                SampleFlags::ZERO,
                1,
                data.to_vec(),
            )
        };
        let audio = |decode_time, data: &[u8]| {
            Sample::new(
                2,
                decode_time,
                1_024,
                0,
                SampleFlags::ZERO,
                1,
                data.to_vec(),
            )
        };

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

    /// The file the chunks make: the brands if any are handed over, one `mdat` per chunk, then the movie
    fn written_file(brands: Option<FileTypeBox>, chunks: Vec<Vec<Sample>>) -> Vec<u8> {
        let mut writer = NonFragmentedWriter::new();
        let mut file = Vec::new();

        if let Some(brands) = brands {
            writer.handle_file_type(brands).unwrap();
        }
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
        let file = written_file(Some(file_type()), two_track_chunks());

        assert_eq!(samples_of(&file, file.len()), two_track_chunks().concat());
    }

    #[test]
    fn each_chunk_is_laid_down_as_its_own_media_data_box_and_the_movie_comes_last() {
        let file = written_file(Some(file_type()), two_track_chunks());

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

    #[test]
    fn a_file_handed_no_brands_is_read_back_declaring_the_brand_its_layout_requires() {
        let file = written_file(None, two_track_chunks());

        let mut reader = NonFragmentedReader::new();
        reader.handle_input(&file).unwrap();

        assert_eq!(
            reader.file_type(),
            Some(&FileTypeBox::new(
                FourCC::new(*b"iso4"),
                0,
                vec![FourCC::new(*b"iso4")]
            ))
        );
    }

    #[test]
    fn the_durations_are_stated_from_the_samples_each_track_was_handed() {
        let epoch = Mp4EpochSeconds::from_seconds(0);
        let template = MovieBox::new(
            MovieHeaderBox::new(epoch, epoch, 1_000, HeaderDuration::ZERO, 3),
            vec![track(1), track(2)],
            None,
        )
        .unwrap();
        let mut writer = NonFragmentedWriter::new();
        let mut file = Vec::new();

        writer.handle_movie(template).unwrap();
        for chunk in two_track_chunks() {
            writer.begin_chunk().unwrap();
            for sample in chunk {
                writer.handle_sample(sample).unwrap();
            }
        }
        writer.finish().unwrap();
        while let Some(written) = writer.poll_output() {
            file.extend_from_slice(&written);
        }
        let mut reader = NonFragmentedReader::new();
        reader.handle_input(&file).unwrap();
        let read = reader.movie().unwrap();

        let mut expected = read.clone();
        let duration = |value| HeaderDuration::new(value).unwrap();
        *expected.mvhd_mut() = read.mvhd().clone().with_duration(duration(100));
        for (track_id, media_duration, track_duration) in [(1, 9_000, 100), (2, 4_096, 46)] {
            let track = expected.trak_mut(track_id).unwrap();
            *track.tkhd_mut() = track.tkhd().clone().with_duration(duration(track_duration));
            *track.mdia_mut().mdhd_mut() = track
                .mdia()
                .mdhd()
                .clone()
                .with_duration(duration(media_duration));
        }
        assert_eq!(read, &expected);
    }

    #[test]
    fn a_file_handed_no_brands_whose_chunk_comes_before_the_movie_is_read_back() {
        let sample = Sample::new(1, 0, 3_000, 0, SampleFlags::ZERO, 1, b"VIDEO_01".to_vec());
        let mut writer = NonFragmentedWriter::new();
        let mut file = Vec::new();

        writer.begin_chunk().unwrap();
        writer.handle_sample(sample.clone()).unwrap();
        writer.handle_movie(movie()).unwrap();
        writer.finish().unwrap();
        while let Some(written) = writer.poll_output() {
            file.extend_from_slice(&written);
        }

        assert_eq!(samples_of(&file, file.len()), [sample]);
    }
}
