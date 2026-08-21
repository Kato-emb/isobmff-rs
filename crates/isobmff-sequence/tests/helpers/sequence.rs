//! A file laid out box by box, and the reader and the writer driven over it
//!
//! Shared across the integration test binaries with
//! `#[path = "helpers/sequence.rs"] pub mod sequence;`.

// Why `pub` on what the binaries import: each of them drives a part of this
// module, and a part left undriven reads as dead code under `pub(crate)`.

use core::ops::Range;
use isobmff_boxes::{
    FileTypeBox, HandlerBox, MediaBox, MediaHeaderBox, MediaInformationBox, MovieBox,
    MovieFragmentBox, MovieFragmentHeaderBox, MovieHeaderBox, SampleDescriptionBox, SampleTableBox,
    SegmentTypeBox, TrackBox, TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentBox,
    TrackFragmentHeaderBox, TrackHeaderBox,
};
use isobmff_core::{
    AnyBox, BoxHeader, BoxSize, BoxType, BoxWrite, FourCC, FullBoxFlags, LanguageCode,
    Mp4EpochSeconds, NullTerminatedString, Uuid,
};

use isobmff_sequence::{BoxEvent, BoxReader, BoxWriter, Error};

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
fn framed(box_type: BoxType, payload: &[u8]) -> Option<Vec<u8>> {
    let header = BoxHeader::with_payload_len(box_type, u64::try_from(payload.len()).ok()?)?;

    Some(laid_out(header, payload))
}

/// The bytes a box occupies, its header and its payload
pub fn written(value: &impl BoxWrite) -> Option<Vec<u8>> {
    let mut bytes = vec![0; usize::try_from(value.encoded_len()).ok()?];
    value.encode(&mut bytes).ok()?;

    Some(bytes)
}

/// Every event a reader reports for `file`, handed over `cut_length` bytes at a time
///
/// # Errors
///
/// The failure the reader reports for the file, the events made before it
/// dropped along with it
pub fn events_of(file: &[u8], cut_length: usize) -> Result<Vec<(Range<u64>, BoxEvent)>, Error> {
    let mut reader = BoxReader::new();
    let mut events = Vec::new();

    for arriving in file.chunks(cut_length) {
        reader.handle_input(arriving)?;
        while let Some(event) = polled(&mut reader) {
            events.push(event);
        }
    }
    reader.finish()?;
    while let Some(event) = polled(&mut reader) {
        events.push(event);
    }

    Ok(events)
}

/// The next event the reader reports, with the bytes of the file it was read from
pub fn polled(reader: &mut BoxReader) -> Option<(Range<u64>, BoxEvent)> {
    let event = reader.poll_event()?;
    let extent = reader.event_extent()?;

    Some((extent, event))
}

/// The steps with the payload of each box passed on fused back into one
///
/// What is left is what the file says rather than how it was cut, each step with
/// the bytes of the file it was read from — the fused payload with the extent of
/// the whole run
pub fn payloads_fused(events: Vec<(Range<u64>, BoxEvent)>) -> Vec<(Range<u64>, BoxEvent)> {
    let mut fused: Vec<(Range<u64>, BoxEvent)> = Vec::new();

    for step in events {
        match (fused.last_mut(), step) {
            (
                Some((gathered_extent, BoxEvent::RawPayload(gathered))),
                (extent, BoxEvent::RawPayload(bytes)),
            ) => {
                gathered_extent.end = extent.end;
                gathered.extend_from_slice(&bytes);
            }
            (_not_two_payloads, step) => fused.push(step),
        }
    }

    fused
}

/// The file a writer lays down for `events`, drained `buffer_length` bytes at a time
///
/// # Errors
///
/// The failure the writer reports for the events, the bytes made before it
/// dropped along with it
pub fn bytes_of(events: Vec<BoxEvent>, buffer_length: usize) -> Result<Vec<u8>, Error> {
    let mut writer = BoxWriter::new();
    let mut buffer = vec![0; buffer_length];
    let mut file = Vec::new();

    for event in events {
        writer.handle_event(event)?;
        drained_into(&mut writer, &mut buffer, &mut file);
    }
    writer.finish()?;
    drained_into(&mut writer, &mut buffer, &mut file);

    Ok(file)
}

/// Drains what the writer has made so far into `file`, through `buffer`
fn drained_into(writer: &mut BoxWriter, buffer: &mut [u8], file: &mut Vec<u8>) {
    loop {
        let written = writer.poll_output(buffer);

        match buffer.get(..written) {
            Some([]) | None => return,
            Some(bytes) => file.extend_from_slice(bytes),
        }
    }
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
pub fn segment_type() -> SegmentTypeBox {
    SegmentTypeBox::new(
        FourCC::new(*b"msdh"),
        0,
        vec![FourCC::new(*b"msdh"), FourCC::new(*b"msix")],
    )
}

/// Movie the synthetic files declare, one track of video
pub fn movie() -> Option<MovieBox> {
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

/// Fragment the synthetic files carry, adding to the track the movie declared
pub fn movie_fragment() -> Option<MovieFragmentBox> {
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
pub fn media_data_header() -> Option<BoxHeader> {
    BoxHeader::with_payload_len(
        BoxType::compact(*b"mdat"),
        u64::try_from(MEDIA_DATA.len()).ok()?,
    )
}

/// A synthetic fragmented file: the brands, the movie, one fragment, its media data
pub fn fragmented_file() -> Option<Vec<u8>> {
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

/// A synthetic segment: the brands of the segment, one fragment, its media data
pub fn segment_file() -> Option<Vec<u8>> {
    Some(
        [
            written(&segment_type())?,
            written(&movie_fragment()?)?,
            framed(BoxType::compact(*b"mdat"), &MEDIA_DATA)?,
        ]
        .concat(),
    )
}

/// A synthetic file of boxes passed on as they lie, its last box running to the end of it
pub fn file_passed_on() -> Option<Vec<u8>> {
    // Why hold no box the reader reads into a value: such a box is reported as
    // that value and never as the bytes it was read from, which is what this file
    // fixes. `value_boxes.rs` covers those.
    let unbounded = BoxHeader::new(BoxType::compact(*b"mdat"), BoxSize::ToEndOfFile)?;
    let mut file = framed(BoxType::compact(*b"free"), b"")?;

    file.extend_from_slice(&framed(BoxType::compact(*b"skip"), &[0xa5; 40])?);
    file.extend_from_slice(&framed(BoxType::Extended(USER_TYPE), b"vendor!!")?);
    file.extend_from_slice(&framed(BoxType::compact(*b"mdat"), &[0x11; 64])?);
    file.extend_from_slice(&laid_out(unbounded, &[0x22; 48]));

    Some(file)
}
