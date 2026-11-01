//! Lists the boxes and the samples of a movie file or a segment, seeking to the bytes the demuxer wants
//!
//! ```text
//! cargo run --example demux -- movie.mp4
//! ```
//!
//! The demuxer names the bytes it wants next. The caller seeks the file there
//! and reads a buffer of it, then passes that buffer again for as long as the
//! demuxer returns events, reading another only once the demuxer wants bytes
//! past it. A read that returns no byte is the end of the file, which the
//! caller tells the demuxer with `finish`.
//!
//! The demuxer skips each `mdat`, and once a `moov` or a `moof` locates
//! samples in it, wants their bytes. The samples come out of `poll_sample`
//! once their bytes have arrived, those whose bytes came with a call that
//! fails among them: the caller takes them before it returns the failure.
//!
//! The media data of a track may be in another file, which a `url ` entry of
//! its data reference names relative to the movie file (ISO/IEC 14496-12
//! §8.7.2). The demuxer locates those samples and does not gather them. Once
//! the movie file is read, the caller opens each file a data reference names,
//! and gathers the samples in it with a `MediaDataReader`, seeking to the
//! bytes it wants as it does for the demuxer. The samples in a file that does
//! not open, or in an entry other than a `url `, are counted and not read, as
//! are those past where a file ends.

use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::iter;
use std::path::{Path, PathBuf};

use isobmff::boxes::DataEntry;
use isobmff::media_data::MediaDataReader;
use isobmff::{ByteRange, DemuxEvent, DemuxLimits, Demuxer, Sample, SampleLocation};

/// Samples listed per track
const LISTED_SAMPLES: usize = 3;

fn main() -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(env::args_os().nth(1).ok_or("usage: demux <file>")?);
    let mut file = BufferedFile::open(&path)?;
    let mut demuxer = Demuxer::new();
    let mut samples = BTreeMap::<u32, Vec<Sample>>::new();
    let mut elsewhere = BTreeMap::<(u32, u16), Vec<SampleLocation>>::new();

    while let Some(wanted) = demuxer.wanted_input() {
        let (buffered_at, input) = file.read_at(wanted.offset())?;
        let event = match input {
            [] => demuxer.finish(),
            input => demuxer.handle_input(buffered_at, input),
        };
        while let Some(sample) = demuxer.poll_sample() {
            samples
                .entry(sample.properties().track_id)
                .or_default()
                .push(sample);
        }
        if let Some(event) = event.inspect_err(|_| list_samples(&samples))? {
            if let DemuxEvent::Movie {
                samples: locations, ..
            }
            | DemuxEvent::MovieFragment {
                samples: locations, ..
            } = &event
            {
                for location in locations.clone().filter(|location| {
                    !matches!(demuxer.data_reference(location), Some(DataEntry::Url(url)) if url.location.is_none())
                }) {
                    elsewhere
                        .entry((
                            location.properties().track_id,
                            location.data_reference_index(),
                        ))
                        .or_default()
                        .push(location);
                }
            }
            report(&demuxer, event)?;
        }
    }

    for ((track_id, data_reference_index), locations) in elsewhere {
        let not_read = format!(
            "track {track_id}: {} samples in data reference entry {data_reference_index}, not all read",
            locations.len()
        );
        let entry = locations
            .first()
            .and_then(|location| demuxer.data_reference(location));
        let Some(DataEntry::Url(url)) = entry else {
            println!("{not_read}: {entry:?}");
            continue;
        };
        let Some(named) = url.location.as_ref().and_then(|named| named.as_str()) else {
            println!("{not_read}: a location that is not UTF-8");
            continue;
        };
        let mut gathered = Vec::new();
        if let Err(error) = gather(&path.with_file_name(named), locations, &mut gathered) {
            println!("{not_read}: {named}: {error}");
        }
        if !gathered.is_empty() {
            samples.entry(track_id).or_default().extend(gathered);
        }
    }

    list_samples(&samples);
    Ok(())
}

/// File read a buffer at a time, from the file offset a reader wants
struct BufferedFile {
    file: File,
    buffer: Vec<u8>,
    /// File offset the buffered bytes start at
    buffered_at: u64,
    /// Bytes buffered, from the start of `buffer`
    buffered: usize,
}

impl BufferedFile {
    /// Opens the file at `path`, with nothing buffered
    fn open(path: &Path) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            file: File::open(path)?,
            buffer: vec![0; 64 * 1024],
            buffered_at: 0,
            buffered: 0,
        })
    }

    /// Returns the buffered bytes and the file offset they start at, reading a buffer from `offset` unless they hold it; no byte at the end of the file
    fn read_at(&mut self, offset: u64) -> Result<(u64, &[u8]), Box<dyn Error>> {
        let buffered_end = self
            .buffered_at
            .saturating_add(u64::try_from(self.buffered)?);
        if !(self.buffered_at..buffered_end).contains(&offset) {
            self.file.seek(SeekFrom::Start(offset))?;
            self.buffered = self.file.read(&mut self.buffer)?;
            self.buffered_at = offset;
        }
        let input = self
            .buffer
            .get(..self.buffered)
            .ok_or("read past the buffer")?;
        Ok((self.buffered_at, input))
    }
}

/// Gathers the samples of `locations` out of the file at `path` into `samples`, seeking to the bytes a media data reader wants
fn gather(
    path: &Path,
    locations: Vec<SampleLocation>,
    samples: &mut Vec<Sample>,
) -> Result<(), Box<dyn Error>> {
    let mut file = BufferedFile::open(path)?;
    let mut media_data = MediaDataReader::new(DemuxLimits::new().media_data());
    media_data.handle_sample_locations(locations)?;
    while let Some(wanted) = media_data.wanted_input() {
        let (buffered_at, input) = file.read_at(wanted.offset())?;
        if input.is_empty() {
            break;
        }
        let handled = media_data.handle_input(buffered_at, input);
        samples.extend(iter::from_fn(|| media_data.poll_sample()));
        handled?;
    }
    media_data.finish()?;
    Ok(())
}

/// Prints the box `event` reports
fn report(demuxer: &Demuxer, event: DemuxEvent) -> Result<(), Box<dyn Error>> {
    match event {
        DemuxEvent::FileType { byte_range, .. } => {
            let ftyp = demuxer.file_type().ok_or("ftyp without its value")?;
            print_box(
                "ftyp",
                byte_range,
                &format!("major brand {}", ftyp.major_brand),
            );
        }
        DemuxEvent::SegmentType { byte_range, .. } => {
            let styp = demuxer.segment_type().ok_or("styp without its value")?;
            print_box(
                "styp",
                byte_range,
                &format!("major brand {}", styp.major_brand),
            );
        }
        DemuxEvent::Movie {
            byte_range,
            samples,
            ..
        } => {
            let tracks = demuxer.movie().map_or(0, |movie| movie.trak.len());
            print_box(
                "moov",
                byte_range,
                &format!("{tracks} tracks, {} samples", samples.len()),
            );
        }
        DemuxEvent::MovieFragment {
            movie_fragment,
            byte_range,
            samples,
            ..
        } => {
            print_box(
                "moof",
                byte_range,
                &format!(
                    "sequence number {}, {} samples",
                    movie_fragment.mfhd.sequence_number,
                    samples.len()
                ),
            );
        }
        DemuxEvent::SegmentIndex { byte_range, .. } => {
            let subsegments = demuxer
                .segment_indexes()
                .iter()
                .find(|index| index.byte_range().start == byte_range.offset())
                .map_or(0, |index| index.subsegments().len());
            print_box("sidx", byte_range, &format!("{subsegments} subsegments"));
        }
        DemuxEvent::MovieFragmentRandomAccess { byte_range, .. } => {
            let entries = demuxer
                .movie_fragment_random_access()
                .iter()
                .flat_map(|mfra| &mfra.tfra)
                .map(|tfra| tfra.entries.len())
                .sum::<usize>();
            print_box("mfra", byte_range, &format!("{entries} entries"));
        }
        DemuxEvent::MediaData {
            header, byte_range, ..
        }
        | DemuxEvent::Other {
            header, byte_range, ..
        } => {
            print_box(&header.box_type().to_string(), byte_range, "skipped");
        }
        _ => {}
    }
    Ok(())
}

/// Prints one line for a box at file level: its type, where it lies, and what was read of it
fn print_box(box_type: &str, byte_range: ByteRange, note: &str) {
    let size = byte_range
        .length()
        .map_or_else(|| "to end of file".to_owned(), |length| length.to_string());
    println!(
        "[{box_type}] offset={} size={size}: {note}",
        byte_range.offset()
    );
}

/// Prints how many samples each track has, and the first few with the first bytes of each
fn list_samples(samples: &BTreeMap<u32, Vec<Sample>>) {
    for (track_id, samples) in samples {
        let bytes: usize = samples.iter().map(|sample| sample.data().len()).sum();
        println!("track {track_id}: {} samples, {bytes} bytes", samples.len());
        for sample in samples.iter().take(LISTED_SAMPLES) {
            let head: Vec<_> = sample
                .data()
                .iter()
                .take(8)
                .map(|byte| format!("{byte:02x}"))
                .collect();
            println!(
                "  decoding time {}, {} bytes: {} ...",
                sample.properties().decoding_time,
                sample.data().len(),
                head.join(" ")
            );
        }
    }
}
