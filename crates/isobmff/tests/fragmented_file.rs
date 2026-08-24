//! The samples of a fragmented file, read through the box layer beside this one

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
        BoxDefinition, BoxEncode, BoxHeader, MediaDataBox, MovieFragmentBox,
        MovieFragmentHeaderBox, Sample, TrackExtendsBox, TrackFragmentBaseMediaDecodeTimeBox,
        TrackFragmentBox, TrackFragmentHeaderBox, TrackRunBox, TrackRunSample,
    };
    use isobmff_test_support::{file_type, fragmented_movie, written};

    /// Bytes each sample of the synthetic file occupies
    const SAMPLE_LEN: usize = 8;

    /// Ticks each sample of the synthetic file lasts
    const SAMPLE_DURATION: u32 = 3_000;

    /// Decode time the fragment of the synthetic file starts at
    const BASE_MEDIA_DECODE_TIME: u64 = 90_000;

    /// Media data the fragment of the synthetic file addresses: three samples
    const MEDIA_DATA: [u8; 24] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16,
        0x17, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27,
    ];

    /// Fragment the file carries, its three samples lying `data_offset` past its start
    fn movie_fragment(data_offset: i32) -> MovieFragmentBox {
        let samples = (0..3)
            .map(|_| TrackRunSample::new(None, None, None, None).unwrap())
            .collect();
        let track_fragment = TrackFragmentBox::new(
            TrackFragmentHeaderBox::new(
                TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF,
                1,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap(),
            Some(TrackFragmentBaseMediaDecodeTimeBox::new(
                BASE_MEDIA_DECODE_TIME,
            )),
            vec![TrackRunBox::new(Some(data_offset), None, samples).unwrap()],
        )
        .unwrap();

        MovieFragmentBox::new(MovieFragmentHeaderBox::new(1), vec![track_fragment])
    }

    /// A synthetic fragmented file: the brands, the movie, one fragment, its media data
    ///
    /// The offsets of the fragment are anchored at the fragment itself, so the run
    /// states where the media data lies past its own start: over the fragment and
    /// the header of the `mdat` beside it.
    fn fragmented_file() -> Vec<u8> {
        let track_extends =
            TrackExtendsBox::new(1, 1, SAMPLE_DURATION, u32::try_from(SAMPLE_LEN).unwrap(), 0);
        let media_data = MediaDataBox::new(MEDIA_DATA.to_vec());
        let header_len = BoxHeader::with_payload_len(
            MediaDataBox::BOX_TYPE,
            u64::try_from(MEDIA_DATA.len()).unwrap(),
        )
        .unwrap()
        .encoded_len();
        let data_offset = i32::try_from(
            movie_fragment(0)
                .encoded_len()
                .saturating_add(u64::try_from(header_len).unwrap()),
        )
        .unwrap();

        [
            written(&file_type()),
            written(&fragmented_movie(track_extends)),
            written(&movie_fragment(data_offset)),
            written(&media_data),
        ]
        .concat()
    }

    /// The samples the synthetic file was built to carry
    fn declared_samples() -> Vec<Sample> {
        let mut decode_time = BASE_MEDIA_DECODE_TIME;

        MEDIA_DATA
            .chunks(SAMPLE_LEN)
            .map(|data| {
                let sample =
                    Sample::new(1, decode_time, SAMPLE_DURATION, None, 0, 1, data.to_vec());
                decode_time = decode_time.saturating_add(u64::from(SAMPLE_DURATION));

                sample
            })
            .collect()
    }

    #[test]
    fn the_samples_of_a_fragmented_file_are_read_through_the_reader_of_its_boxes() {
        let file = fragmented_file();

        assert_eq!(samples_of(&file, file.len()), declared_samples());
    }

    #[test]
    fn the_samples_are_the_same_however_the_file_was_cut() {
        let file = fragmented_file();

        for cut_length in [1, 3, 7, 64, file.len().saturating_sub(1)] {
            assert_eq!(samples_of(&file, cut_length), declared_samples());
        }
    }
}
