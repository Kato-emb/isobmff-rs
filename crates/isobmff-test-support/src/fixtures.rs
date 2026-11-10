//! Movies built from the fields of their boxes, which the tests of reading and writing them share

use isobmff_boxes::{
    ChunkLargeOffsetBox, ChunkOffsetBox, ChunkOffsetVariant, DataEntry, DataEntryUrlBox, MovieBox,
    MovieExtendsBox, MovieFragmentBox, SampleDescription, SampleEntry, SampleSizeBox,
    SampleSizeVariant, SampleSizes, SampleToChunkBox, SampleToChunkEntry, TimeToSampleBox,
    TimeToSampleEntry, TrackBox, TrackExtendsBox, TrackFragmentBox, TrackRunBox, TrackRunSamples,
};
use isobmff_core::{
    BoxDefinition, BoxEncode, BoxHeader, BoxSize, BoxType, ExtendedSize, Utf8String,
};

/// Returns the track `track_id` of the samples of `sizes`, each lasting `sample_delta`, laid in chunks at `chunk_offsets` by the runs `(first_chunk, samples_per_chunk)`
///
/// The track has one sample entry, whose media data is in the file itself.
///
/// # Panics
///
/// * `sizes` holds more samples than a `u32` counts.
#[must_use]
#[expect(
    clippy::unwrap_used,
    reason = "a test of more samples than a sample table counts fails"
)]
pub fn track(
    track_id: u32,
    sample_delta: u32,
    sizes: &[u32],
    runs: &[(u32, u32)],
    chunk_offsets: &[u64],
) -> TrackBox {
    let mut trak = TrackBox::default();
    trak.tkhd.track_id = track_id;
    let stbl = &mut trak.mdia.minf.stbl;
    let mut entry = SampleEntry::default();
    entry.data_reference_index = 1;
    stbl.stsd.sample_descriptions = Vec::from([SampleDescription::Typed(entry)]);
    let mut stts = TimeToSampleBox::default();
    let mut run = TimeToSampleEntry::default();
    run.sample_count = u32::try_from(sizes.len()).unwrap();
    run.sample_delta = sample_delta;
    stts.entries = Vec::from([run]);
    stbl.stts = Some(stts);
    let mut stsc = SampleToChunkBox::default();
    stsc.entries = runs
        .iter()
        .map(|&(first_chunk, samples_per_chunk)| {
            let mut entry = SampleToChunkEntry::default();
            entry.first_chunk = first_chunk;
            entry.samples_per_chunk = samples_per_chunk;
            entry.sample_description_index = 1;
            entry
        })
        .collect();
    stbl.stsc = Some(stsc);
    let mut stsz = SampleSizeBox::default();
    stsz.sample_sizes = SampleSizes::Table {
        entry_sizes: sizes.to_vec(),
    };
    stbl.sample_size = Some(SampleSizeVariant::SampleSize(stsz));
    stbl.chunk_offset = Some(
        match chunk_offsets
            .iter()
            .map(|&offset| u32::try_from(offset))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(compact) => {
                let mut stco = ChunkOffsetBox::default();
                stco.chunk_offsets = compact;
                ChunkOffsetVariant::ChunkOffset(stco)
            }
            Err(_) => {
                let mut co64 = ChunkLargeOffsetBox::default();
                co64.chunk_offsets = chunk_offsets.to_vec();
                ChunkOffsetVariant::ChunkLargeOffset(co64)
            }
        },
    );
    trak.mdia.minf.dinf.dref.data_entries = Vec::from([DataEntry::Url(DataEntryUrlBox::default())]);
    trak
}

/// Returns `trak` with its media data in the file `other.mp4`, which its only data reference entry names
#[must_use]
pub fn in_another_file(mut trak: TrackBox) -> TrackBox {
    let mut url = DataEntryUrlBox::default();
    url.location = Some(Utf8String::from_slice(b"other.mp4"));
    trak.mdia.minf.dinf.dref.data_entries = Vec::from([DataEntry::Url(url)]);
    trak
}

/// Returns `value` written as a box
///
/// # Panics
///
/// * `value` does not write.
#[must_use]
#[expect(
    clippy::unwrap_used,
    reason = "a test of a box that does not write fails"
)]
pub fn encoded(value: &(impl BoxEncode + BoxDefinition)) -> Vec<u8> {
    let mut bytes = vec![0; usize::try_from(value.encoded_len()).unwrap()];
    value.encode(&mut bytes).unwrap();
    bytes
}

/// Returns the header of an `mdat` of `payload_length` bytes, 16 bytes with a `largesize`, or `None` where the box would be longer than 64 bits count
#[must_use]
pub fn media_data_header(payload_length: u64) -> Option<[u8; 16]> {
    let size = ExtendedSize::new(payload_length.checked_add(16)?)?;
    let header = BoxHeader::new(BoxType::compact(*b"mdat"), BoxSize::Extended(size))?;
    let mut bytes = [0; 16];
    header.encode(&mut bytes).ok()?;
    Some(bytes)
}

/// Returns the box of `box_type` whose payload is `fields` laid end to end, under a 32-bit size
///
/// # Panics
///
/// * The box is longer than a 32-bit size counts.
#[must_use]
#[expect(
    clippy::unwrap_used,
    reason = "a test of a box longer than a 32-bit size fails"
)]
pub fn written(box_type: &[u8; 4], fields: &[&[u8]]) -> Vec<u8> {
    let payload = fields.concat();
    let size = u32::try_from(payload.len().checked_add(8).unwrap()).unwrap();
    [&size.to_be_bytes()[..], box_type, &payload].concat()
}

/// Returns the movie of `tracks`
#[must_use]
pub fn movie(tracks: impl IntoIterator<Item = TrackBox>) -> MovieBox {
    let mut movie = MovieBox::default();
    movie.trak = tracks.into_iter().collect();
    movie
}

/// Returns a movie file whose `count` movie fragments come before the samples they declare
///
/// The file holds track 1, whose `moov` declares no sample. Each of the
/// `count` fragments that follow declares one sample, and every one of
/// those samples lies in one `mdat` after the last fragment. One more
/// fragment then follows, its sample in an `mdat` after it.
///
/// Each sample is 4 bytes lasting 20, its sample entry the first, as the
/// `trex` of the track gives them. The fragments give no `tfdt`, so the
/// decoding time of each sample is the sum of the durations of the samples
/// resolved before it.
///
/// # Panics
///
/// * A length passes what a `u64` counts.
#[must_use]
#[expect(
    clippy::unwrap_used,
    reason = "a test of a file longer than a u64 counts fails"
)]
pub fn fragments_ahead_of_their_samples(count: usize) -> Vec<u8> {
    let mut movie = movie([track(1, 20, &[], &[], &[])]);
    let mut trex = TrackExtendsBox::default();
    trex.track_id = 1;
    trex.default_sample_description_index = 1;
    trex.default_sample_duration = 20;
    trex.default_sample_size = 4;
    let mut mvex = MovieExtendsBox::default();
    mvex.trex = Vec::from([trex]);
    movie.mvex = Some(mvex);
    movie.mvhd.next_track_id = 2;
    let moof = |base_data_offset: u64| {
        let mut traf = TrackFragmentBox::default();
        traf.tfhd.track_id = 1;
        traf.tfhd.base_data_offset = Some(base_data_offset);
        let mut trun = TrackRunBox::default();
        trun.samples = TrackRunSamples::EmptyRows { sample_count: 1 };
        traf.trun = Vec::from([trun]);
        let mut moof = MovieFragmentBox::default();
        moof.traf = Vec::from([traf]);
        encoded(&moof)
    };
    let length = |length: usize| u64::try_from(length).unwrap();
    let moof_length = length(moof(0).len());
    let samples_length = length(count).checked_mul(4).unwrap();
    let mut file = encoded(&movie);
    let first_sample = length(count)
        .checked_mul(moof_length)
        .and_then(|moofs| moofs.checked_add(length(file.len())))
        .and_then(|end| end.checked_add(8))
        .unwrap();
    for index in 0..length(count) {
        file.extend(moof(
            first_sample
                .checked_add(index.checked_mul(4).unwrap())
                .unwrap(),
        ));
    }
    file.extend(written(
        b"mdat",
        &[&vec![0xAA; usize::try_from(samples_length).unwrap()]],
    ));
    let last_sample = length(file.len())
        .checked_add(moof_length)
        .and_then(|end| end.checked_add(8))
        .unwrap();
    file.extend(moof(last_sample));
    file.extend(written(b"mdat", &[&[0xBB; 4]]));
    file
}
