//! The synthetic files that carry samples, laid out box by box, and the samples each was built to carry
//!
//! A file here states its sample tables or its runs by hand rather than
//! through a writer of this workspace, so what a reader makes of it is held
//! against a list written out beside it, not against what a writer was handed.

use alloc::vec;
use alloc::vec::Vec;
use core::num::NonZeroU32;

use isobmff_boxes::{
    MediaDataBox, MovieBox, MovieFragmentBox, MovieFragmentHeaderBox, MovieFragmentRandomAccessBox,
    ReferenceType, SampleFlags, SegmentIndexBox, SegmentIndexReference, TrackExtendsBox,
    TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentBox, TrackFragmentHeaderBox,
    TrackFragmentHeaderFlags, TrackFragmentRandomAccessBox, TrackFragmentRandomAccessEntry,
    TrackRunBox, TrackRunSample,
};
use isobmff_core::{BoxDefinition, BoxEncode, BoxHeader};
use isobmff_sample::Sample;
use isobmff_sequence::BoxEvent;

use crate::boxes::{
    SAMPLE_DURATION, TIMESCALE, file_type, fragmented_movie, segment_type, written,
};
use crate::driving::events_of;

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
/// the header of the `mdat` beside it. The fragment states its decode time in
/// a `tfdt` where one is given.
fn fragment_over(
    sequence_number: u32,
    decode_time: Option<u64>,
    media_data: &[u8],
) -> MovieFragmentBox {
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
        );
        let track_fragment = match decode_time {
            Some(decode_time) => {
                track_fragment.with_tfdt(TrackFragmentBaseMediaDecodeTimeBox::new(decode_time))
            }
            None => track_fragment,
        };

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
            let sample = Sample::new(
                1,
                decode_time,
                SAMPLE_DURATION,
                0,
                SampleFlags::ZERO,
                1,
                data.to_vec(),
            );
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
        SampleFlags::ZERO,
    ))
}

/// A fragmented file laid out by hand: the brands, the movie, one fragment, its media data
pub fn fragmented_file_with_samples() -> Vec<u8> {
    let media_data = FRAGMENTED_MEDIA_DATA.as_slice();

    [
        written(&file_type()),
        written(&presentation_movie()),
        written(&fragment_over(1, Some(BASE_MEDIA_DECODE_TIME), media_data)),
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
            Some(decode_time),
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
            let sample = Sample::new(
                1,
                decode_time,
                SAMPLE_DURATION,
                0,
                SampleFlags::ZERO,
                1,
                data.to_vec(),
            );
            decode_time = decode_time.saturating_add(u64::from(SAMPLE_DURATION));

            sample
        })
        .collect()
}

/// A file laid out by hand with indexes, and where its fragments lie
///
/// The fragments carry the media data of [`segment_file_with_samples`], one
/// `moof` and one `mdat` each.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct IndexedFile {
    /// The bytes of the file
    pub bytes: Vec<u8>,
    /// Where each `moof` starts in the file, in the order they lie
    pub moof_offsets: Vec<u64>,
    /// The samples each fragment carries, in the order the fragments lie
    pub fragment_samples: Vec<Vec<Sample>>,
}

/// Lays out `head`, then a `sidx` indexing the fragments that follow it, then the fragments
///
/// The `sidx` references each fragment whole from the first byte after
/// itself, starting at the earliest presentation time of the first. The
/// fragments state their decode times in a `tfdt` where `with_decode_times`
/// says so; the timeline starts at 90 000 where they do, and at zero where
/// they do not, as a reader of such a file places its first sample
/// (§8.8.12). The samples, the `sidx` and the fragments share that start. The
/// offsets are checked against the bytes: every `moof` the file frames lies
/// where they say.
fn indexed(head: Vec<u8>, with_decode_times: bool) -> IndexedFile {
    let earliest_decode_time = if with_decode_times {
        BASE_MEDIA_DECODE_TIME
    } else {
        0
    };
    let mut decode_time = earliest_decode_time;
    let mut fragments = Vec::new();
    let mut fragment_samples = Vec::new();
    let mut references = Vec::new();
    for (position, media_data) in SEGMENT_MEDIA_DATA.iter().enumerate() {
        let sequence_number = u32::try_from(position).unwrap().saturating_add(1);
        let fragment = [
            written(&fragment_over(
                sequence_number,
                with_decode_times.then_some(decode_time),
                media_data,
            )),
            written(&MediaDataBox::new(media_data.to_vec())),
        ]
        .concat();
        let samples = samples_over([*media_data], decode_time);
        let duration = SAMPLE_DURATION.saturating_mul(u32::try_from(samples.len()).unwrap());

        references.push(
            SegmentIndexReference::new(
                ReferenceType::MediaContent,
                u32::try_from(fragment.len()).unwrap(),
                duration,
                true,
                1,
                0,
            )
            .unwrap(),
        );
        decode_time = decode_time.saturating_add(u64::from(duration));
        fragments.push(fragment);
        fragment_samples.push(samples);
    }
    let segment_index =
        written(&SegmentIndexBox::new(1, TIMESCALE, earliest_decode_time, 0, references).unwrap());

    let mut moof_offsets = Vec::new();
    let mut bytes = [head, segment_index].concat();
    for fragment in fragments {
        moof_offsets.push(u64::try_from(bytes.len()).unwrap());
        bytes.extend_from_slice(&fragment);
    }

    let framed_moof_offsets: Vec<u64> = events_of(&bytes, bytes.len())
        .unwrap()
        .into_iter()
        .filter_map(|(extent, event)| {
            if let BoxEvent::Header(header) = event {
                (header.box_type() == MovieFragmentBox::BOX_TYPE).then_some(extent.start)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(framed_moof_offsets, moof_offsets);

    IndexedFile {
        bytes,
        moof_offsets,
        fragment_samples,
    }
}

/// Appends to `file` the `mfra` of track 1, listing the first sample of each of its fragments as a sync sample
fn with_random_access(mut file: IndexedFile) -> IndexedFile {
    let entries = file
        .moof_offsets
        .iter()
        .zip(&file.fragment_samples)
        .map(|(&moof_offset, samples)| {
            TrackFragmentRandomAccessEntry::new(
                samples.first().unwrap().decode_time(),
                moof_offset,
                NonZeroU32::MIN,
                NonZeroU32::MIN,
                NonZeroU32::MIN,
            )
        })
        .collect();

    file.bytes
        .extend_from_slice(&written(&MovieFragmentRandomAccessBox::new(vec![
            TrackFragmentRandomAccessBox::new(1, entries),
        ])));

    file
}

/// A fragmented file laid out by hand with both indexes: `ftyp moov sidx moof mdat moof mdat mfra`
pub fn indexed_fragmented_file() -> IndexedFile {
    with_random_access(indexed(
        [written(&file_type()), written(&presentation_movie())].concat(),
        true,
    ))
}

/// [`indexed_fragmented_file`] with fragments stating no `tfdt`, so their decode times follow only from the fragments before them
pub fn indexed_fragmented_file_without_decode_times() -> IndexedFile {
    with_random_access(indexed(
        [written(&file_type()), written(&presentation_movie())].concat(),
        false,
    ))
}

/// A media segment laid out by hand with its index: `styp sidx moof mdat moof mdat`
pub fn indexed_segment_file() -> IndexedFile {
    indexed(written(&segment_type()), true)
}
