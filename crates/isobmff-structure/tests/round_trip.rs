//! The samples a writer laid down as a fragmented movie file, read back off that file

// Why not inside `mod tests`: an inline `mod` adds its own name as a directory
// segment, so a nested one looks for `tests/tests/helpers/reading.rs`. The
// `cfg` is what keeps `allow-unwrap-in-tests` reaching the helper from out here.
#[cfg(test)]
#[path = "helpers/reading.rs"]
mod reading;

#[cfg(test)]
mod tests {
    use super::reading::samples_of;
    use isobmff_boxes::{
        MovieBox, MovieExtendsBox, MovieHeaderBox, SampleFlags, TrackBox, TrackExtendsBox,
    };
    use isobmff_core::{AnyBox, BoxType, Mp4EpochSeconds};
    use isobmff_sample::Sample;
    use isobmff_structure::FragmentedWriter;
    use isobmff_test_support::{EVERY_FIELD_AT_ITS_HIGHEST, file_type, track};

    /// Ticks a second the media of the movie is timed in
    const TIMESCALE: u32 = 90_000;

    /// Movie of two tracks continued in fragments
    ///
    /// Every default the `trex` boxes state is one no fragment this writer lays
    /// out falls back on: a `tfhd` states its own. A sample read back holding one
    /// of these values would mean the fragment left it to the movie.
    fn movie() -> MovieBox {
        let epoch = Mp4EpochSeconds::from_seconds(0);
        let never_fallen_back_on =
            |track_id| TrackExtendsBox::new(track_id, 9, 1, 1, EVERY_FIELD_AT_ITS_HIGHEST);

        MovieBox::new(
            MovieHeaderBox::new(epoch, epoch, TIMESCALE, 0, 3),
            vec![track(1), track(2)],
            MovieExtendsBox::new(vec![never_fallen_back_on(1), never_fallen_back_on(2)]),
        )
        .unwrap()
    }

    /// The samples the two tracks carry, fragment by fragment
    ///
    /// The video track holds three samples in the first fragment, interleaved
    /// with the audio track so its own run is broken in two, and states flags
    /// only its first sample differs on. The audio samples state composition time
    /// offsets, positive in one fragment and negative in the other, which the two
    /// versions of a `trun` write apart.
    fn two_track_fragments() -> Vec<Vec<Sample>> {
        let video = |decode_time, sample_flags, data: &[u8]| {
            Sample::new(1, decode_time, 3_000, 0, sample_flags, 1, data.to_vec())
        };
        let audio = |decode_time, offset, data: &[u8]| {
            Sample::new(
                2,
                decode_time,
                1_024,
                offset,
                SampleFlags::ZERO,
                1,
                data.to_vec(),
            )
        };

        vec![
            vec![
                video(0, SampleFlags::SYNC_SAMPLE, b"VIDEO_01"),
                video(3_000, SampleFlags::NON_SYNC_SAMPLE, b"VIDEO_02"),
                audio(0, 512, b"AUD1"),
                video(6_000, SampleFlags::NON_SYNC_SAMPLE, b"VIDEO_03"),
            ],
            vec![
                video(9_000, SampleFlags::SYNC_SAMPLE, b"VIDEO_04"),
                audio(1_024, -256, b"AUD2"),
            ],
        ]
    }

    /// The file the samples make: the brands, the movie, then fragment after fragment
    fn written_file(movie: MovieBox, fragments: Vec<Vec<Sample>>) -> Vec<u8> {
        let mut writer = FragmentedWriter::new();
        let mut file = Vec::new();

        writer.handle_file_type(file_type()).unwrap();
        writer.handle_movie(movie).unwrap();

        for (position, samples) in fragments.into_iter().enumerate() {
            let sequence_number = u32::try_from(position).unwrap().saturating_add(1);

            writer.begin_fragment(sequence_number).unwrap();
            for sample in samples {
                writer.handle_sample(sample).unwrap();
            }
            writer.finish_fragment().unwrap();
        }
        writer.finish().unwrap();

        while let Some(written) = writer.poll_output() {
            file.extend_from_slice(&written);
        }

        file
    }

    #[test]
    fn the_samples_are_read_back_as_they_were_handed_over_however_the_file_was_cut() {
        let file = written_file(movie(), two_track_fragments());
        let handed_over = two_track_fragments().concat();

        for cut_length in [file.len(), 1, 3, 7, 64, file.len().saturating_sub(1)] {
            assert_eq!(
                samples_of(&file, cut_length),
                handed_over,
                "cut at {cut_length}"
            );
        }
    }

    #[test]
    fn a_sync_sample_of_a_video_track_filled_for_fragments_is_read_back() {
        let entry =
            AnyBox::from_raw_bytes(BoxType::compact(*b"avc1"), vec![0, 0, 0, 0, 0, 0, 0, 1]);
        let track = TrackBox::new_video(1, TIMESCALE, 1920, 1080, entry);
        let sample = Sample::new(
            1,
            0,
            3_000,
            0,
            SampleFlags::SYNC_SAMPLE,
            1,
            b"VIDEO_01".to_vec(),
        );

        let file = written_file(
            MovieBox::new_fragmented(TIMESCALE, vec![track]).unwrap(),
            vec![vec![sample.clone()]],
        );

        assert_eq!(samples_of(&file, file.len()), [sample]);
    }
}
