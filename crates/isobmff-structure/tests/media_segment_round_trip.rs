//! The samples a writer laid down as a media segment, read back off that segment

// Why not inside `mod tests`: an inline `mod` adds its own name as a directory
// segment, so a nested one looks for `tests/tests/helpers/media_segment_reading.rs`.
// The `cfg` is what keeps `allow-unwrap-in-tests` reaching the helper from out here.
#[cfg(test)]
#[path = "helpers/media_segment_reading.rs"]
mod reading;

#[cfg(test)]
mod tests {
    use super::reading::samples_of;
    use isobmff_boxes::{
        DegradationPriorityEntry, MovieBox, MovieExtendsBox, MovieHeaderBox, PaddingBitsEntry,
        SampleDependencyTypeEntry, SampleFlags, TrackExtendsBox,
    };
    use isobmff_core::Mp4EpochSeconds;
    use isobmff_sample::Sample;
    use isobmff_structure::MediaSegmentWriter;
    use isobmff_test_support::{segment_type, track};

    /// Ticks a second the media of the movie is timed in
    const TIMESCALE: u32 = 90_000;

    /// Flags stating every field a `sample_flags` word carries at its highest value
    const EVERY_FIELD_AT_ITS_HIGHEST: SampleFlags = SampleFlags::new(
        SampleDependencyTypeEntry::new(3, 3, 3, 3).unwrap(),
        PaddingBitsEntry::new(7).unwrap(),
        true,
        DegradationPriorityEntry::new(u16::MAX),
    );

    /// Movie of two tracks the segment continues, which no fragment falls back on
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

    /// The samples the two tracks carry, fragment by fragment, the tracks interleaved in the first
    ///
    /// The segment starts partway into the presentation, so the first sample of
    /// each track states a decode time no fragment before it led up to. The
    /// audio sample of the first fragment lies between two video samples, so
    /// the fragment declares its samples in another order than they lie in.
    fn two_track_fragments() -> Vec<Vec<Sample>> {
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
                video(90_000, b"VIDEO_01"),
                audio(30_720, 512, b"AUD1"),
                video(93_000, b"VIDEO_02"),
            ],
            vec![video(96_000, b"VIDEO_03"), audio(31_744, -256, b"AUD2")],
        ]
    }

    /// The segment the samples make: the brands, then fragment after fragment
    fn written_segment(fragments: Vec<Vec<Sample>>) -> Vec<u8> {
        let mut writer = MediaSegmentWriter::new();
        let mut segment = Vec::new();

        writer.handle_segment_type(segment_type()).unwrap();

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
            segment.extend_from_slice(&written);
        }

        segment
    }

    #[test]
    fn the_samples_are_read_back_as_they_were_handed_over_however_the_segment_was_cut() {
        let segment = written_segment(two_track_fragments());

        for cut_length in [segment.len(), 1, 3, 7, 64, segment.len().saturating_sub(1)] {
            assert_eq!(
                samples_of(movie(), &segment, cut_length),
                two_track_fragments().concat(),
                "cut at {cut_length}"
            );
        }
    }
}
