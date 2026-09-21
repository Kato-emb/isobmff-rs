//! The samples of a media segment laid out by hand, read back through its structure against the movie it continues

// Why not inside `mod tests`: an inline `mod` adds its own name as a directory
// segment, so a nested one looks for `tests/tests/helpers/media_segment_reading.rs`.
// The `cfg` is what keeps `allow-unwrap-in-tests` reaching the helper from out here.
#[cfg(test)]
#[path = "helpers/media_segment_reading.rs"]
mod reading;

#[cfg(test)]
mod tests {
    use super::reading::samples_of;
    use isobmff::{
        BoxDefinition, BoxEncode, BoxHeader, MediaDataBox, MovieBox, MovieFragmentBox,
        MovieFragmentHeaderBox, Sample, TrackExtendsBox, TrackFragmentBaseMediaDecodeTimeBox,
        TrackFragmentBox, TrackFragmentHeaderBox, TrackFragmentHeaderFlags, TrackRunBox,
        TrackRunSample,
    };
    use isobmff_test_support::{SAMPLE_DURATION, fragmented_movie, segment_type, written};
    #[cfg(feature = "std")]
    use {isobmff::MediaSegmentDemuxer, std::io};

    /// Bytes each sample of the synthetic segment occupies
    const SAMPLE_LEN: usize = 8;

    /// Decode time the first fragment of the synthetic segment starts at
    const BASE_MEDIA_DECODE_TIME: u64 = 90_000;

    /// Media data the fragments of the synthetic segment address: three samples, then two
    const MEDIA_DATA: [&[u8]; 2] = [
        &[
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15,
            0x16, 0x17, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27,
        ],
        &[
            0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x40, 0x41, 0x42, 0x43, 0x44, 0x45,
            0x46, 0x47,
        ],
    ];

    /// The movie the synthetic segment continues, its `trex` stating what every sample shares
    fn movie() -> MovieBox {
        fragmented_movie(TrackExtendsBox::new(
            1,
            1,
            SAMPLE_DURATION,
            u32::try_from(SAMPLE_LEN).unwrap(),
            0,
        ))
    }

    /// A fragment declaring one sample per `SAMPLE_LEN` of `media_data`, lying `data_offset` past its start
    fn movie_fragment(
        sequence_number: u32,
        decode_time: u64,
        media_data: &[u8],
        data_offset: i32,
    ) -> MovieFragmentBox {
        let samples = media_data
            .chunks(SAMPLE_LEN)
            .map(|_sample| TrackRunSample::new(None, None, None, None))
            .collect();
        let track_fragment = TrackFragmentBox::new(
            TrackFragmentHeaderBox::new(
                TrackFragmentHeaderFlags::DEFAULT_BASE_IS_MOOF,
                1,
                None,
                None,
                None,
                None,
                None,
            ),
            Some(TrackFragmentBaseMediaDecodeTimeBox::new(decode_time)),
            vec![TrackRunBox::new(Some(data_offset), None, samples).unwrap()],
        );

        MovieFragmentBox::new(
            MovieFragmentHeaderBox::new(sequence_number),
            vec![track_fragment],
        )
    }

    /// A synthetic media segment: the brands, then two fragments each with its media data
    ///
    /// The offsets of a fragment are anchored at the fragment itself, so each run
    /// states where the media data lies past its own start: over the fragment and
    /// the header of the `mdat` beside it.
    fn media_segment() -> Vec<u8> {
        let mut segment = written(&segment_type());
        let mut decode_time = BASE_MEDIA_DECODE_TIME;

        for (position, media_data) in MEDIA_DATA.iter().enumerate() {
            let sequence_number = u32::try_from(position).unwrap().saturating_add(1);
            let header_length = BoxHeader::with_payload_len(
                MediaDataBox::BOX_TYPE,
                u64::try_from(media_data.len()).unwrap(),
            )
            .unwrap()
            .encoded_len();
            let data_offset = i32::try_from(
                movie_fragment(sequence_number, decode_time, media_data, 0)
                    .encoded_len()
                    .saturating_add(u64::try_from(header_length).unwrap()),
            )
            .unwrap();

            segment.extend_from_slice(&written(&movie_fragment(
                sequence_number,
                decode_time,
                media_data,
                data_offset,
            )));
            segment.extend_from_slice(&written(&MediaDataBox::new(media_data.to_vec())));
            decode_time = decode_time.saturating_add(
                u64::from(SAMPLE_DURATION).saturating_mul((media_data.len() / SAMPLE_LEN) as u64),
            );
        }

        segment
    }

    /// The samples the synthetic segment was built to carry
    fn declared_samples() -> Vec<Sample> {
        let mut decode_time = BASE_MEDIA_DECODE_TIME;

        MEDIA_DATA
            .iter()
            .flat_map(|media_data| media_data.chunks(SAMPLE_LEN))
            .map(|data| {
                let sample = Sample::new(1, decode_time, SAMPLE_DURATION, 0, 0, 1, data.to_vec());
                decode_time = decode_time.saturating_add(u64::from(SAMPLE_DURATION));

                sample
            })
            .collect()
    }

    #[test]
    fn the_samples_of_a_media_segment_are_read_off_the_bytes_it_lies_as() {
        let segment = media_segment();

        assert_eq!(
            samples_of(movie(), &segment, segment.len()),
            declared_samples()
        );
    }

    #[test]
    fn the_samples_are_the_same_however_the_segment_was_cut() {
        let segment = media_segment();

        for cut_length in [1, 3, 7, 64, segment.len().saturating_sub(1)] {
            assert_eq!(
                samples_of(movie(), &segment, cut_length),
                declared_samples()
            );
        }
    }

    #[cfg(feature = "std")]
    #[test]
    fn a_source_that_seeks_has_every_sample_read_off_it() {
        let read_back: Vec<Sample> =
            MediaSegmentDemuxer::new(io::Cursor::new(media_segment()), movie())
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();

        assert_eq!(read_back, declared_samples());
    }
}
