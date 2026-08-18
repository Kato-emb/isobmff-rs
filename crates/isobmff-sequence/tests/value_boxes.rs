//! Boxes a file is framed by come back out of the reader as the values they hold

#![allow(
    clippy::tests_outside_test_module,
    reason = "an integration test binary ships no items, so its tests are the crate root"
)]

#[path = "helpers/reading.rs"]
mod reading;

use isobmff_boxes::{
    FileTypeBox, HandlerBox, MediaBox, MediaHeaderBox, MediaInformationBox, MovieBox,
    MovieFragmentBox, MovieFragmentHeaderBox, MovieHeaderBox, SampleDescriptionBox, SampleTableBox,
    TrackBox, TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentBox, TrackFragmentHeaderBox,
    TrackHeaderBox,
};
use isobmff_core::{
    AnyBox, BoxEncode as _, BoxHeader, BoxType, BoxWrite, FourCC, FullBoxFlags, LanguageCode,
    NullTerminatedString, QuickTimeDateTime,
};
use isobmff_sequence::{BoxEvent, BoxReader, BoxReaderError};
use reading::{events_of, framed, payloads_fused};

/// Time every header of the synthetic file declares
const EPOCH: QuickTimeDateTime = QuickTimeDateTime::from_seconds(0);

/// Ticks a second the media of the synthetic file is timed in
const TIMESCALE: u32 = 90_000;

/// Media data the fragment of the synthetic file addresses
const MEDIA_DATA: [u8; 64] = [0x11; 64];

/// Brands the synthetic file declares itself readable as
fn file_type() -> FileTypeBox {
    FileTypeBox::new(
        FourCC::new(*b"iso6"),
        512,
        vec![FourCC::new(*b"iso6"), FourCC::new(*b"dash")],
    )
}

/// Movie the synthetic file declares, one track of video
fn movie() -> Option<MovieBox> {
    let sample_description = SampleDescriptionBox::new(vec![AnyBox::from_raw_bytes(
        BoxType::compact(*b"avc1"),
        vec![0xab; 4],
    )]);
    let media = MediaBox::new(
        MediaHeaderBox::new(EPOCH, EPOCH, TIMESCALE, 0, LanguageCode::UND),
        HandlerBox::new(
            FourCC::new(*b"vide"),
            NullTerminatedString::new(String::from("VideoHandler"))?,
        ),
        MediaInformationBox::new(SampleTableBox::new(sample_description)),
    );
    let track = TrackBox::new(
        TrackHeaderBox::new(FullBoxFlags::new(1)?, EPOCH, EPOCH, 1, 0),
        media,
    );

    MovieBox::new(
        MovieHeaderBox::new(EPOCH, EPOCH, TIMESCALE, 0, 2),
        vec![track],
        None,
    )
}

/// Fragment the synthetic file carries, adding to the track the movie declared
fn movie_fragment() -> Option<MovieFragmentBox> {
    let track_fragment = TrackFragmentBox::new(
        TrackFragmentHeaderBox::new(FullBoxFlags::ZERO, 1, None, None, None, None, None)?,
        Some(TrackFragmentBaseMediaDecodeTimeBox::new(0)),
        Vec::new(),
    )?;

    Some(MovieFragmentBox::new(
        MovieFragmentHeaderBox::new(1),
        vec![track_fragment],
    ))
}

/// Header of the box the media data is framed as
fn media_data_header() -> Option<BoxHeader> {
    BoxHeader::with_payload_len(
        BoxType::compact(*b"mdat"),
        u64::try_from(MEDIA_DATA.len()).ok()?,
    )
}

/// The bytes a box occupies, its header and its payload
fn written(value: &impl BoxWrite) -> Option<Vec<u8>> {
    let mut bytes = vec![0; usize::try_from(value.encoded_len()).ok()?];
    value.encode(&mut bytes).ok()?;

    Some(bytes)
}

/// A synthetic fragmented file: the brands, the movie, one fragment, its media data
fn fragmented_file() -> Option<Vec<u8>> {
    Some(
        [
            written(&file_type())?,
            written(&movie()?)?,
            written(&movie_fragment()?)?,
            framed(BoxType::compact(*b"mdat"), &MEDIA_DATA)?,
        ]
        .concat(),
    )
}

#[test]
fn the_boxes_a_file_is_framed_by_are_values_and_its_media_data_is_passed_on() {
    let movie = movie().unwrap();
    let movie_fragment = movie_fragment().unwrap();
    let movie_at = file_type().encoded_len();
    let fragment_at = movie_at.saturating_add(movie.encoded_len());
    let media_data_at = fragment_at.saturating_add(movie_fragment.encoded_len());
    let file = fragmented_file().unwrap();

    assert_eq!(
        events_of(&file, file.len()).unwrap(),
        vec![
            BoxEvent::FileType {
                ftyp: file_type(),
                file_offset: 0
            },
            BoxEvent::Movie {
                moov: movie,
                file_offset: movie_at
            },
            BoxEvent::MovieFragment {
                moof: movie_fragment,
                file_offset: fragment_at
            },
            BoxEvent::RawStart {
                header: media_data_header().unwrap(),
                file_offset: media_data_at
            },
            BoxEvent::RawPayload(Vec::from(MEDIA_DATA)),
            BoxEvent::RawEnd,
        ]
    );
}

#[test]
fn the_events_reported_do_not_turn_on_where_the_fragmented_file_was_cut() {
    let file = fragmented_file().unwrap();
    let whole = payloads_fused(events_of(&file, file.len()).unwrap());

    for cut_length in 1..=file.len() {
        assert_eq!(
            payloads_fused(events_of(&file, cut_length).unwrap()),
            whole,
            "cut every {cut_length} bytes"
        );
    }
}

#[test]
fn a_value_declaring_more_than_the_limit_stops_the_reader_where_it_stands() {
    let declared = movie().unwrap().payload_len();
    let limit = declared.saturating_sub(1);
    let file = fragmented_file().unwrap();
    let mut reader = BoxReader::with_payload_limit(limit);

    assert!(matches!(
        reader.handle_read(&file),
        Err(BoxReaderError::PayloadLimitExceeded {
            box_type,
            declared: reported_declared,
            limit: reported_limit
        }) if box_type == BoxType::compact(*b"moov")
            && reported_declared == declared
            && reported_limit == limit
    ));
    assert_eq!(
        reader.poll_event(),
        Some(BoxEvent::FileType {
            ftyp: file_type(),
            file_offset: 0
        })
    );
    assert_eq!(reader.poll_event(), None);
    assert!(matches!(
        reader.handle_read(&file),
        Err(BoxReaderError::AlreadyFailed)
    ));
}
