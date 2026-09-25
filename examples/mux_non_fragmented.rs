//! Writes a non-fragmented MP4 file of one audio track, a synthesized 440 Hz square wave
//!
//! The movie is built box by box: 48 kHz mono 16-bit big-endian PCM under the QuickTime sample
//! entry `twos`, 1024 frames a sample and the last one short, one chunk a second, 3 seconds unless
//! `seconds` says otherwise.
//!
//! Usage: `cargo run -p isobmff-examples --example mux_non_fragmented -- <out.mp4> [seconds]`

use core::error::Error;
use std::env;
use std::fs::File;
use std::io::BufWriter;
use std::time::{SystemTime, UNIX_EPOCH};

use isobmff::boxes::{
    AudioSampleEntry, ChunkOffsetBox, ChunkOffsets, DataEntry, DataEntryUrlBox, DataInformationBox,
    DataReferenceBox, HandlerBox, HeaderDuration, MediaBox, MediaHeaderBox, MediaInformationBox,
    MediaInformationHeader, MovieBox, MovieHeaderBox, SampleDescriptionBox, SampleFlags,
    SampleSizeBox, SampleSizeEntries, SampleSizes, SampleTableBox, SampleToChunkBox,
    SoundMediaHeaderBox, TimeToSampleBox, TrackBox, TrackHeaderBox,
};
use isobmff::core::{
    AnyBox, BoxType, FieldWriter, FourCC, FullBoxFlags, I8F8, LanguageCode, Mp4EpochSeconds,
    NullTerminatedString, U16F16,
};
use isobmff::io::blocking::NonFragmentedMuxer;
use isobmff::sample::Sample;

const SAMPLE_RATE: u64 = 48_000;
const FREQUENCY: u64 = 440;
const AMPLITUDE: i16 = 8_192;
const FRAMES_PER_SAMPLE: u64 = 1_024;
const TRACK_ID: u32 = 1;

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let path = arguments
        .next()
        .ok_or("usage: mux_non_fragmented <out.mp4> [seconds]")?;
    let seconds: u64 = arguments.next().map_or(Ok(3), |seconds| seconds.parse())?;
    let frames = seconds.checked_mul(SAMPLE_RATE).ok_or("too many seconds")?;

    let mut entry = vec![0; usize::try_from(AudioSampleEntry::LEN)?];
    let mut fields = FieldWriter::new(&mut entry);
    AudioSampleEntry::new(1, U16F16::from_integer(u16::try_from(SAMPLE_RATE)?))
        .with_channel_count(1)
        .with_sample_size(16)
        .encode_fields(&mut fields)?;
    fields.finish()?;
    let sample_table = SampleTableBox::new(
        SampleDescriptionBox::new(vec![AnyBox::from_raw_bytes(
            BoxType::compact(*b"twos"),
            entry,
        )]),
        TimeToSampleBox::new(Vec::new()),
        SampleToChunkBox::new(Vec::new()),
        SampleSizes::Stsz(SampleSizeBox::new(SampleSizeEntries::PerSample(Vec::new()))),
        ChunkOffsets::Stco(ChunkOffsetBox::new(Vec::new())),
    );
    let media_information = MediaInformationBox::new(
        MediaInformationHeader::Sound(SoundMediaHeaderBox::new()),
        DataInformationBox::new(DataReferenceBox::new(vec![DataEntry::Url(
            DataEntryUrlBox::new(None),
        )])),
        sample_table,
    );
    let now =
        Mp4EpochSeconds::from_unix_seconds(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
            .ok_or("the clock is past what a header states")?;
    let duration = HeaderDuration::new(frames).ok_or("too many seconds")?;
    let handler_name =
        NullTerminatedString::new(String::from("SoundHandler")).ok_or("a NUL in the name")?;
    let media = MediaBox::new(
        MediaHeaderBox::new(
            now,
            now,
            u32::try_from(SAMPLE_RATE)?,
            duration,
            LanguageCode::UND,
        ),
        HandlerBox::new(FourCC::new(*b"soun"), handler_name),
        media_information,
    );
    let enabled_in_movie = FullBoxFlags::new(0x3).ok_or("flags past 24 bits")?;
    let track_header = TrackHeaderBox::new(
        enabled_in_movie,
        now,
        now,
        TRACK_ID,
        duration,
        U16F16::ZERO,
        U16F16::ZERO,
    )
    .with_volume(I8F8::ONE);
    let movie_header = MovieHeaderBox::new(now, now, u32::try_from(SAMPLE_RATE)?, duration, 2);
    let movie = MovieBox::new(movie_header, vec![TrackBox::new(track_header, media)], None)
        .ok_or("the movie declares no track")?;

    let mut muxer = NonFragmentedMuxer::new(BufWriter::new(File::create(path)?));
    muxer.handle_movie(movie)?;
    for start in (0..frames).step_by(usize::try_from(FRAMES_PER_SAMPLE)?) {
        let end = start.saturating_add(FRAMES_PER_SAMPLE).min(frames);
        if start % SAMPLE_RATE < FRAMES_PER_SAMPLE {
            muxer.begin_chunk()?;
        }
        let data = (start..end)
            .flat_map(|frame| {
                let half_periods = frame.saturating_mul(2 * FREQUENCY) / SAMPLE_RATE;
                let level = if half_periods % 2 == 0 {
                    AMPLITUDE
                } else {
                    -AMPLITUDE
                };
                level.to_be_bytes()
            })
            .collect();
        let duration = u32::try_from(end.saturating_sub(start))?;
        muxer.handle_sample(Sample::new(
            TRACK_ID,
            start,
            duration,
            0,
            SampleFlags::ZERO,
            1,
            data,
        ))?;
    }
    muxer.finish()?;

    Ok(())
}
