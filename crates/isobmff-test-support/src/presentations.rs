//! The synthetic files that carry samples, laid out box by box, and the samples each was built to carry
//!
//! A file here states its sample tables or its runs by hand rather than
//! through a writer of this workspace, so what a reader makes of it is held
//! against a list written out beside it, not against what a writer was handed.

use alloc::vec;
use alloc::vec::Vec;

use isobmff_boxes::{
    MediaDataBox, MovieBox, MovieFragmentBox, MovieFragmentHeaderBox, TrackExtendsBox,
    TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentBox, TrackFragmentHeaderBox,
    TrackFragmentHeaderFlags, TrackRunBox, TrackRunSample,
};
use isobmff_core::{BoxDefinition, BoxEncode, BoxHeader};
use isobmff_sample::Sample;

use crate::boxes::{SAMPLE_DURATION, file_type, fragmented_movie, segment_type, written};

/// Bytes each sample of these files occupies
const SAMPLE_LEN: usize = 8;

/// Decode time the first fragment of these files starts at
const BASE_MEDIA_DECODE_TIME: u64 = 90_000;

/// Media data the fragment of [`fragmented_file_with_samples`] addresses: three samples
const FRAGMENTED_MEDIA_DATA: [u8; 24] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
    0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27,
];

/// Media data the fragments of [`segment_file_with_samples`] address: three samples, then two
const SEGMENT_MEDIA_DATA: [&[u8]; 2] = [
    &[
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16,
        0x17, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27,
    ],
    &[
        0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46,
        0x47,
    ],
];

/// The samples a non-fragmented file is laid out from, chunk by chunk
pub const SAMPLE_CHUNKS: [&[&[u8]]; 3] = [
    &[b"SAMPLE_1", b"SAMPLE_2"],
    &[b"SAMPLE_3"],
    &[b"SAMPLE_4", b"SAMPLE_5", b"SAMPLE_6"],
];

/// A fragment declaring one sample per [`SAMPLE_LEN`] of the media data lying beside it
///
/// The offsets of a fragment are anchored at the fragment itself, so its run
/// states where the media data lies past its own start: over the fragment and
/// the header of the `mdat` beside it.
fn fragment_over(sequence_number: u32, decode_time: u64, media_data: &[u8]) -> MovieFragmentBox {
    let laid_out = |data_offset| {
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
            vec![TrackRunBox::new(Some(data_offset), None, samples).unwrap()],
        )
        .with_tfdt(TrackFragmentBaseMediaDecodeTimeBox::new(decode_time));

        MovieFragmentBox::new(
            MovieFragmentHeaderBox::new(sequence_number),
            vec![track_fragment],
        )
    };
    let media_data_header = BoxHeader::with_payload_len(
        MediaDataBox::BOX_TYPE,
        u64::try_from(media_data.len()).unwrap(),
    )
    .unwrap()
    .encoded_len();
    let data_offset = i32::try_from(
        laid_out(0)
            .encoded_len()
            .saturating_add(u64::try_from(media_data_header).unwrap()),
    )
    .unwrap();

    laid_out(data_offset)
}

/// The samples `media_data` carries, one per [`SAMPLE_LEN`], from `decode_time` on
fn samples_over<'data>(
    media_data: impl IntoIterator<Item = &'data [u8]>,
    mut decode_time: u64,
) -> Vec<Sample> {
    media_data
        .into_iter()
        .flat_map(|data| data.chunks(SAMPLE_LEN))
        .map(|data| {
            let sample = Sample::new(1, decode_time, SAMPLE_DURATION, 0, 0, 1, data.to_vec());
            decode_time = decode_time.saturating_add(u64::from(SAMPLE_DURATION));

            sample
        })
        .collect()
}

/// The movie the fragments of these files continue, its `trex` stating what every sample shares
pub fn presentation_movie() -> MovieBox {
    fragmented_movie(TrackExtendsBox::new(
        1,
        1,
        SAMPLE_DURATION,
        u32::try_from(SAMPLE_LEN).unwrap(),
        0,
    ))
}

/// A fragmented file laid out by hand: the brands, the movie, one fragment, its media data
pub fn fragmented_file_with_samples() -> Vec<u8> {
    let media_data = FRAGMENTED_MEDIA_DATA.as_slice();

    [
        written(&file_type()),
        written(&presentation_movie()),
        written(&fragment_over(1, BASE_MEDIA_DECODE_TIME, media_data)),
        written(&MediaDataBox::new(media_data.to_vec())),
    ]
    .concat()
}

/// The samples [`fragmented_file_with_samples`] was built to carry
pub fn fragmented_file_samples() -> Vec<Sample> {
    samples_over([FRAGMENTED_MEDIA_DATA.as_slice()], BASE_MEDIA_DECODE_TIME)
}

/// A media segment laid out by hand: the brands, then two fragments each with its media data
pub fn segment_file_with_samples() -> Vec<u8> {
    let mut segment = written(&segment_type());
    let mut decode_time = BASE_MEDIA_DECODE_TIME;

    for (position, media_data) in SEGMENT_MEDIA_DATA.iter().enumerate() {
        let sequence_number = u32::try_from(position).unwrap().saturating_add(1);

        segment.extend_from_slice(&written(&fragment_over(
            sequence_number,
            decode_time,
            media_data,
        )));
        segment.extend_from_slice(&written(&MediaDataBox::new(media_data.to_vec())));
        decode_time = decode_time.saturating_add(
            u64::from(SAMPLE_DURATION)
                .saturating_mul(u64::try_from(media_data.len() / SAMPLE_LEN).unwrap()),
        );
    }

    segment
}

/// The samples [`segment_file_with_samples`] was built to carry
pub fn segment_file_samples() -> Vec<Sample> {
    samples_over(SEGMENT_MEDIA_DATA, BASE_MEDIA_DECODE_TIME)
}

/// The samples a non-fragmented file of [`SAMPLE_CHUNKS`] was built to carry
pub fn non_fragmented_file_samples() -> Vec<Sample> {
    let mut decode_time = 0;

    SAMPLE_CHUNKS
        .iter()
        .flat_map(|chunk| chunk.iter())
        .map(|data| {
            let sample = Sample::new(1, decode_time, SAMPLE_DURATION, 0, 0, 1, data.to_vec());
            decode_time = decode_time.saturating_add(u64::from(SAMPLE_DURATION));

            sample
        })
        .collect()
}
