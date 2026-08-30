//! The samples a writer laid out as fragments, read back off the file they make

// Why not inside `mod tests`: an inline `mod` adds its own name as a directory
// segment, so a nested one looks for `tests/tests/helpers/reading.rs`. The
// `cfg` is what keeps `allow-unwrap-in-tests` reaching the helper from out here.
#[cfg(test)]
#[path = "helpers/reading.rs"]
mod reading;

#[cfg(test)]
mod tests {
    use super::reading::samples_of;
    use isobmff::{
        BoxDefinition, BoxHeader, MediaDataBox, MovieBox, MovieExtendsBox, MovieHeaderBox,
        Mp4EpochSeconds, Sample, SampleWriter, TrackExtendsBox,
    };
    use isobmff_sequence::BoxEvent;
    use isobmff_test_support::{bytes_of, file_type, track};

    /// Ticks a second the media of the movie is timed in
    const TIMESCALE: u32 = 90_000;

    /// Flags the samples of the video track state, but for the first of a fragment
    const NOT_A_SYNC_SAMPLE: u32 = 0x0101_0000;

    /// Flags the first sample of a fragment of the video track states
    const SYNC_SAMPLE: u32 = 0x0200_0000;

    /// Movie of two tracks continued in fragments
    ///
    /// Every default the `trex` boxes state is one no fragment this writer lays
    /// out falls back on: a `tfhd` states its own. A sample read back holding one
    /// of these values would mean the fragment left it to the movie.
    fn movie() -> MovieBox {
        let epoch = Mp4EpochSeconds::from_seconds(0);
        let never_fallen_back_on = |track_id| TrackExtendsBox::new(track_id, 9, 1, 1, u32::MAX);

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
    fn declared_samples() -> Vec<Vec<Sample>> {
        let video = |decode_time, sample_flags, data: &[u8]| {
            Sample::new(1, decode_time, 3_000, None, sample_flags, 1, data.to_vec())
        };
        let audio = |decode_time, offset, data: &[u8]| {
            Sample::new(2, decode_time, 1_024, Some(offset), 0, 1, data.to_vec())
        };

        vec![
            vec![
                video(0, SYNC_SAMPLE, b"VIDEO_01"),
                video(3_000, NOT_A_SYNC_SAMPLE, b"VIDEO_02"),
                audio(0, 512, b"AUD1"),
                video(6_000, NOT_A_SYNC_SAMPLE, b"VIDEO_03"),
            ],
            vec![
                video(9_000, SYNC_SAMPLE, b"VIDEO_04"),
                audio(1_024, -256, b"AUD2"),
            ],
        ]
    }

    /// The file the samples make: the brands, the movie, then fragment after fragment
    fn written_file(fragments: Vec<Vec<Sample>>) -> Vec<u8> {
        let mut writer = SampleWriter::new();
        let mut events = vec![BoxEvent::FileType(file_type()), BoxEvent::Movie(movie())];

        for (position, samples) in fragments.into_iter().enumerate() {
            let sequence_number = u32::try_from(position).unwrap().saturating_add(1);

            writer.begin_fragment(sequence_number).unwrap();
            for sample in samples {
                writer.handle_sample(sample).unwrap();
            }
            writer.finish_fragment().unwrap();

            while let Some((movie_fragment, media_data)) = writer.poll_fragment() {
                let payload_len = u64::try_from(media_data.data().len()).unwrap();
                let header =
                    BoxHeader::with_payload_len(MediaDataBox::BOX_TYPE, payload_len).unwrap();

                events.push(BoxEvent::MovieFragment(movie_fragment));
                events.push(BoxEvent::RawStart(header));
                events.push(BoxEvent::RawPayload(media_data.into_data()));
                events.push(BoxEvent::RawEnd);
            }
        }
        writer.finish().unwrap();

        bytes_of(events, 64).unwrap()
    }

    /// The samples of `track_id`, in the order they lie in `samples`
    fn of_track(samples: &[Sample], track_id: u32) -> Vec<Sample> {
        samples
            .iter()
            .filter(|sample| sample.track_id() == track_id)
            .cloned()
            .collect()
    }

    #[test]
    fn the_samples_of_each_track_are_read_back_as_they_were_handed_over() {
        let file = written_file(declared_samples());
        let read_back = samples_of(&file, file.len());
        let handed_over = declared_samples().concat();

        assert_eq!(of_track(&read_back, 1), of_track(&handed_over, 1));
        assert_eq!(of_track(&read_back, 2), of_track(&handed_over, 2));
        assert_eq!(read_back.len(), handed_over.len());
    }

    #[test]
    fn the_samples_of_one_fragment_are_read_back_track_by_track() {
        let file = written_file(declared_samples());
        let tracks: Vec<u32> = samples_of(&file, file.len())
            .iter()
            .map(Sample::track_id)
            .collect();

        assert_eq!(tracks, [1, 1, 1, 2, 1, 2]);
    }

    #[test]
    fn the_samples_are_read_back_the_same_however_the_file_was_cut() {
        let file = written_file(declared_samples());
        let whole = samples_of(&file, file.len());

        for cut_length in [1, 3, 7, 64, file.len().saturating_sub(1)] {
            assert_eq!(samples_of(&file, cut_length), whole);
        }
    }
}
