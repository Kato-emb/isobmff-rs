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
    TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentBox, TrackFragmentHeaderBox,
    TrackFragmentHeaderFlags, TrackHeaderBox, VideoMediaHeaderBox,
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

/// Ticks each sample of the synthetic files lasts
pub const SAMPLE_DURATION: u32 = 3_000;

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
pub fn framed(box_type: BoxType, payload: &[u8]) -> Vec<u8> {
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
/// The `track_id` is the one field to pin, and the sample entry states a
/// `data_reference_index` of 1, the file itself, and nothing past it. The rest
/// — the handler, the flags, the durations — is filler no caller may read
/// anything into, and the sample tables are empty. A test that turns on one of
/// those states it itself rather than reaching for this.
pub fn track(track_id: u32) -> TrackBox {
    track_described_by(track_id, sample_entry())
}

/// Track of [`track`], its samples described by the one `stsd` entry given
pub fn track_described_by(track_id: u32, entry: AnyBox) -> TrackBox {
    track_laid_out(
        track_id,
        self_contained_data_reference(),
        empty_sample_table(entry),
    )
}

/// Track of [`track`], its media lying in the resources `dref` names, and declaring no sample
pub fn track_reading_from(track_id: u32, dref: DataReferenceBox) -> TrackBox {
    track_laid_out(track_id, dref, empty_sample_table(sample_entry()))
}

/// Track of [`track`], its media lying in the resources `dref` names and laid out by `stbl`
pub fn track_laid_out(track_id: u32, dref: DataReferenceBox, stbl: SampleTableBox) -> TrackBox {
    let media = MediaBox::new(
        MediaHeaderBox::new(EPOCH, EPOCH, TIMESCALE, 0, LanguageCode::UND),
        HandlerBox::new(
            FourCC::new(*b"vide"),
            NullTerminatedString::new(String::from("VideoHandler")).unwrap(),
        ),
        MediaInformationBox::new(
            MediaInformationHeader::Video(VideoMediaHeaderBox::new(0, [0; 3])),
            DataInformationBox::new(dref),
            stbl,
        ),
    );

    TrackBox::new(
        TrackHeaderBox::new(FullBoxFlags::new(1).unwrap(), EPOCH, EPOCH, track_id, 0),
        media,
    )
}

/// Sample table describing its samples by the one `stsd` entry of [`track`], laid out by the four tables given
pub fn sample_table(
    stts: TimeToSampleBox,
    stsc: SampleToChunkBox,
    stsz: SampleSizeBox,
    stco: ChunkOffsetBox,
) -> SampleTableBox {
    SampleTableBox::new(
        SampleDescriptionBox::new(vec![sample_entry()]),
        stts,
        stsc,
        stsz,
        stco,
    )
}

/// The `dref` of [`track`]: one entry, the file itself
pub fn self_contained_data_reference() -> DataReferenceBox {
    DataReferenceBox::new(vec![DataEntry::Url(DataEntryUrlBox::new(None))])
}

/// A `dref` of one entry, a file other than the one carrying the movie
pub fn external_data_reference() -> DataReferenceBox {
    DataReferenceBox::new(vec![DataEntry::Url(DataEntryUrlBox::new(Some(
        NullTerminatedString::new(String::from("media.bin")).unwrap(),
    )))])
}

/// The `stsd` entry of [`track`]: a `data_reference_index` of 1, and nothing past it
fn sample_entry() -> AnyBox {
    AnyBox::from_raw_bytes(BoxType::compact(*b"avc1"), vec![0, 0, 0, 0, 0, 0, 0, 1])
}

/// Sample table describing its samples by `entry` and declaring none
fn empty_sample_table(entry: AnyBox) -> SampleTableBox {
    SampleTableBox::new(
        SampleDescriptionBox::new(vec![entry]),
        TimeToSampleBox::new(Vec::new()),
        SampleToChunkBox::new(Vec::new()),
        SampleSizeBox::new(SampleSizes::PerSample(Vec::new())),
        ChunkOffsetBox::new(Vec::new()),
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
        TrackFragmentHeaderBox::new(
            TrackFragmentHeaderFlags::ZERO,
            1,
            None,
            None,
            None,
            None,
            None,
        ),
        Some(TrackFragmentBaseMediaDecodeTimeBox::new(0)),
        Vec::new(),
    );

    MovieFragmentBox::new(MovieFragmentHeaderBox::new(1), vec![track_fragment])
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

/// A synthetic non-fragmented file of one track: the brands, the movie, and one `mdat` per chunk
///
/// `chunks` holds the bytes of each sample, chunk by chunk, and every sample
/// lasts [`SAMPLE_DURATION`] ticks. The movie lies before the media data where
/// `movie_first` is set and after it otherwise, and declares the same samples
/// either way, each chunk offset naming where that layout put the chunk: a
/// reader of the two files meets the same sample tables, once ahead of the
/// bytes they name and once behind them.
pub fn non_fragmented_file(chunks: &[&[&[u8]]], movie_first: bool) -> Vec<u8> {
    let brands = written(&file_type());
    let media_data: Vec<MediaDataBox> = chunks
        .iter()
        .map(|chunk| MediaDataBox::new(chunk.concat()))
        .collect();
    let sizes: Vec<u32> = chunks
        .iter()
        .flat_map(|chunk| chunk.iter())
        .map(|sample| u32::try_from(sample.len()).unwrap())
        .collect();
    let samples_per_chunk: Vec<(u64, u32)> = chunks
        .iter()
        .map(|chunk| (u64::try_from(chunk.len()).unwrap(), 1))
        .collect();
    let movie_declaring = |chunk_offsets: Vec<u64>| {
        let stbl = sample_table(
            TimeToSampleBox::from_deltas(sizes.iter().map(|_size| SAMPLE_DURATION)),
            SampleToChunkBox::from_chunks(samples_per_chunk.iter().copied()).unwrap(),
            SampleSizeBox::from_sizes(sizes.iter().copied()),
            ChunkOffsetBox::from_offsets(chunk_offsets).unwrap(),
        );

        MovieBox::new(
            MovieHeaderBox::new(EPOCH, EPOCH, TIMESCALE, 0, 2),
            vec![track_laid_out(1, self_contained_data_reference(), stbl)],
            None,
        )
        .unwrap()
    };
    let mut chunk_start = u64::try_from(brands.len()).unwrap();
    if movie_first {
        // Why not building the movie once: the chunk offsets it declares lie
        // past the movie itself, so its length is needed before its offsets
        // are, and the offsets are held in fields of a fixed width, so the
        // length is the same whatever they hold.
        let movie_len = movie_declaring(vec![0; chunks.len()]).encoded_len();
        chunk_start = chunk_start.saturating_add(movie_len);
    }
    let mut chunk_offsets = Vec::new();
    for mdat in &media_data {
        let header_len = mdat.encoded_len().saturating_sub(mdat.payload_len());
        chunk_offsets.push(chunk_start.saturating_add(header_len));
        chunk_start = chunk_start.saturating_add(mdat.encoded_len());
    }
    let movie = written(&movie_declaring(chunk_offsets));
    let media_data = media_data.iter().map(written).collect::<Vec<_>>().concat();

    if movie_first {
        [brands, movie, media_data].concat()
    } else {
        [brands, media_data, movie].concat()
    }
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

/// A synthetic file of boxes no specification here reads, its last running to the end of it
pub fn file_running_to_its_end() -> Vec<u8> {
    let unbounded = BoxHeader::new(BoxType::compact(*b"mdat"), BoxSize::ToEndOfFile).unwrap();
    let mut file = framed(BoxType::compact(*b"free"), b"");

    file.extend_from_slice(&framed(BoxType::compact(*b"skip"), &[0xa5; 40]));
    file.extend_from_slice(&framed(BoxType::Extended(USER_TYPE), b"vendor!!"));
    file.extend_from_slice(&written(&MediaDataBox::new(MEDIA_DATA.to_vec())));
    file.extend_from_slice(&laid_out(unbounded, &[0x22; 48]));

    file
}
