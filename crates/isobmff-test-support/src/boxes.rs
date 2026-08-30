//! The boxes a synthetic file declares, and the files laid out from them

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use isobmff_boxes::{
    ChunkOffsetBox, DataEntry, DataEntryUrlBox, DataInformationBox, DataReferenceBox, FileTypeBox,
    HandlerBox, MediaBox, MediaDataBox, MediaHeaderBox, MediaInformationBox,
    MediaInformationHeader, MovieBox, MovieExtendsBox, MovieFragmentBox, MovieFragmentHeaderBox,
    MovieHeaderBox, SampleDescriptionBox, SampleSizeBox, SampleSizes, SampleTableBox,
    SampleToChunkBox, SegmentTypeBox, TimeToSampleBox, TrackBox, TrackExtendsBox,
    TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentBox, TrackFragmentHeaderBox, TrackHeaderBox,
    VideoMediaHeaderBox,
};
use isobmff_core::{
    AnyBox, BoxDefinition, BoxEncode, BoxHeader, BoxSize, BoxType, FourCC, FullBoxFlags,
    LanguageCode, Mp4EpochSeconds, NullTerminatedString, Uuid,
};

/// Time every header of the synthetic files declares
const EPOCH: Mp4EpochSeconds = Mp4EpochSeconds::from_seconds(0);

/// Ticks a second the media of the synthetic files is timed in
const TIMESCALE: u32 = 90_000;

/// Media data the fragment of the synthetic files addresses
pub const MEDIA_DATA: [u8; 64] = [0x11; 64];

/// User type the vendor box of the file of boxes passed on is declared under
const USER_TYPE: Uuid = Uuid::new([
    0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10,
]);

/// Lays out the box `header` introduces: the header, then `payload`
fn laid_out(header: BoxHeader, payload: &[u8]) -> Vec<u8> {
    let mut buffer = [0; BoxHeader::MAX_ENCODED_LEN];
    let mut bytes = header.encode(&mut buffer).to_vec();

    bytes.extend_from_slice(payload);

    bytes
}

/// Lays out one whole box: the header that `box_type` and `payload` need, then the payload
fn framed(box_type: BoxType, payload: &[u8]) -> Vec<u8> {
    let header =
        BoxHeader::with_payload_len(box_type, u64::try_from(payload.len()).unwrap()).unwrap();

    laid_out(header, payload)
}

/// The bytes a box occupies, its header and its payload
pub fn written(value: &(impl BoxDefinition + BoxEncode)) -> Vec<u8> {
    let mut bytes = vec![0; usize::try_from(value.encoded_len()).unwrap()];
    value.encode(&mut bytes).unwrap();

    bytes
}

/// Brands a file declares itself readable as
pub fn file_type() -> FileTypeBox {
    FileTypeBox::new(
        FourCC::new(*b"iso6"),
        512,
        vec![FourCC::new(*b"iso6"), FourCC::new(*b"dash")],
    )
}

/// Brands a segment of a fragmented file declares itself readable as
fn segment_type() -> SegmentTypeBox {
    SegmentTypeBox::new(
        FourCC::new(*b"msdh"),
        0,
        vec![FourCC::new(*b"msdh"), FourCC::new(*b"msix")],
    )
}

/// Track of video the synthetic movies declare, holding no sample of its own
///
/// The `track_id` is the one field to pin: the sample tables are empty, and the
/// rest — the handler, the flags, the sample entry, the durations — is filler no
/// caller may read anything into. A test that turns on one of those states it
/// itself rather than reaching for this.
pub fn track(track_id: u32) -> TrackBox {
    let sample_description = SampleDescriptionBox::new(vec![AnyBox::from_raw_bytes(
        BoxType::compact(*b"avc1"),
        vec![0xab; 4],
    )]);
    let media = MediaBox::new(
        MediaHeaderBox::new(EPOCH, EPOCH, TIMESCALE, 0, LanguageCode::UND),
        HandlerBox::new(
            FourCC::new(*b"vide"),
            NullTerminatedString::new(String::from("VideoHandler")).unwrap(),
        ),
        MediaInformationBox::new(
            MediaInformationHeader::Video(VideoMediaHeaderBox::new(0, [0; 3])),
            DataInformationBox::new(DataReferenceBox::new(vec![DataEntry::Url(
                DataEntryUrlBox::new(None),
            )])),
            SampleTableBox::new(
                sample_description,
                TimeToSampleBox::new(Vec::new()),
                SampleToChunkBox::new(Vec::new()),
                SampleSizeBox::new(SampleSizes::PerSample(Vec::new())),
                ChunkOffsetBox::new(Vec::new()),
            ),
        ),
    );

    TrackBox::new(
        TrackHeaderBox::new(FullBoxFlags::new(1).unwrap(), EPOCH, EPOCH, track_id, 0),
        media,
    )
}

/// Movie of one track that no `trex` states the defaults of a fragment for
pub fn unfragmented_movie() -> MovieBox {
    MovieBox::new(
        MovieHeaderBox::new(EPOCH, EPOCH, TIMESCALE, 0, 2),
        vec![track(1)],
        None,
    )
    .unwrap()
}

/// Movie of one track continued in fragments, which fall back on `trex`
///
/// The track takes the id `trex` names, so the two cannot state different ones.
pub fn fragmented_movie(trex: TrackExtendsBox) -> MovieBox {
    MovieBox::new(
        MovieHeaderBox::new(EPOCH, EPOCH, TIMESCALE, 0, 2),
        vec![track(trex.track_id())],
        MovieExtendsBox::new(vec![trex]),
    )
    .unwrap()
}

/// Fragment adding time to the track the movie declared, and no sample
pub fn movie_fragment() -> MovieFragmentBox {
    let track_fragment = TrackFragmentBox::new(
        TrackFragmentHeaderBox::new(FullBoxFlags::ZERO, 1, None, None, None, None, None).unwrap(),
        Some(TrackFragmentBaseMediaDecodeTimeBox::new(0)),
        Vec::new(),
    )
    .unwrap();

    MovieFragmentBox::new(MovieFragmentHeaderBox::new(1), vec![track_fragment])
}

/// Header of the box the media data is framed as
pub fn media_data_header() -> BoxHeader {
    BoxHeader::with_payload_len(
        BoxType::compact(*b"mdat"),
        u64::try_from(MEDIA_DATA.len()).unwrap(),
    )
    .unwrap()
}

/// A synthetic fragmented file: the brands, the movie, one fragment, its media data
///
/// The movie declares no `mvex`, which §8.8.1 has for a presentation continued in
/// fragments. The box layer never reads it, so the file is enough to frame; a
/// reader of the samples themselves needs [`fragmented_movie`].
pub fn fragmented_file() -> Vec<u8> {
    [
        written(&file_type()),
        written(&unfragmented_movie()),
        written(&movie_fragment()),
        written(&MediaDataBox::new(MEDIA_DATA.to_vec())),
    ]
    .concat()
}

/// A synthetic segment: the brands of the segment, one fragment, its media data
pub fn segment_file() -> Vec<u8> {
    [
        written(&segment_type()),
        written(&movie_fragment()),
        written(&MediaDataBox::new(MEDIA_DATA.to_vec())),
    ]
    .concat()
}

/// A synthetic file of boxes passed on as they lie, its last box running to the end of it
pub fn file_passed_on() -> Vec<u8> {
    let unbounded = BoxHeader::new(BoxType::compact(*b"mdat"), BoxSize::ToEndOfFile).unwrap();
    let mut file = framed(BoxType::compact(*b"free"), b"");

    file.extend_from_slice(&framed(BoxType::compact(*b"skip"), &[0xa5; 40]));
    file.extend_from_slice(&framed(BoxType::Extended(USER_TYPE), b"vendor!!"));
    file.extend_from_slice(&written(&MediaDataBox::new(MEDIA_DATA.to_vec())));
    file.extend_from_slice(&laid_out(unbounded, &[0x22; 48]));

    file
}
