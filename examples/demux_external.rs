//! Lists the samples of a movie file whose media data is in other files too, reading each file a data reference names
//!
//! ```text
//! cargo run --example demux_external -- movie.mp4
//! ```
//!
//! The media data of a track may be in another file, which a `url ` entry of
//! its data reference names by a URL relative to the movie file (ISO/IEC
//! 14496-12 §8.7.2). The demuxer gathers the samples in the movie file
//! itself, and locates the others without gathering them. The location of
//! each sample carries the key of its data reference entry, so the caller
//! keeps the locations of the others by key while the movie file is read.
//!
//! Once it is read, the caller opens the file the entry of each key names,
//! taking its location as a path relative to the directory of the movie
//! file, and gathers the samples in it with a `MediaDataReader` of its own,
//! seeking to the bytes the reader wants as it does for the demuxer. A
//! location with a scheme (`file:`, `http:`), an entry other than a `url `,
//! and a file that does not open are reported, and their samples are not
//! read.

use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use isobmff::boxes::DataEntry;
use isobmff::media_data::MediaDataReader;
use isobmff::{DataReferenceKey, DemuxEvent, DemuxLimits, Demuxer, SampleLocation};

fn main() -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(
        env::args_os()
            .nth(1)
            .ok_or("usage: demux_external <file>")?,
    );
    let mut file = BufferedFile::open(&path)?;
    let mut demuxer = Demuxer::new();
    let (mut samples_here, mut bytes_here) = (0_usize, 0_usize);
    let mut by_key = BTreeMap::<DataReferenceKey, Vec<SampleLocation>>::new();

    while let Some(wanted) = demuxer.wanted_input() {
        let (buffered_at, input) = file.read_at(wanted.offset())?;
        let event = match input {
            [] => demuxer.finish(),
            input => demuxer.handle_input(buffered_at, input),
        };
        while let Some(sample) = demuxer.poll_sample() {
            samples_here = samples_here.saturating_add(1);
            bytes_here = bytes_here.saturating_add(sample.data().len());
        }
        if let Some(DemuxEvent::Movie { samples, .. } | DemuxEvent::MovieFragment { samples, .. }) =
            event?
        {
            for location in samples.filter(|location| {
                demuxer
                    .data_reference(location.data_reference())
                    .is_some_and(|data_reference| !data_reference.entry().is_in_same_file())
            }) {
                by_key
                    .entry(location.data_reference())
                    .or_default()
                    .push(location);
            }
        }
    }
    println!("in the file itself: {samples_here} samples, {bytes_here} bytes");

    for (key, locations) in by_key {
        let data_reference = demuxer.data_reference(key).ok_or("a key names no entry")?;
        let track_id = data_reference.track_id();
        let named = if let DataEntry::Url(url) = data_reference.entry() {
            url.location.as_ref().and_then(|location| location.as_str())
        } else {
            None
        };
        let Some(named) = named.filter(|named| !named.contains(':')) else {
            println!(
                "track {track_id}: {} samples in {:?}, not read",
                locations.len(),
                data_reference.entry()
            );
            continue;
        };
        match gather(&path.with_file_name(named), locations) {
            Ok((samples, bytes)) => {
                println!("track {track_id} in {named}: {samples} samples, {bytes} bytes");
            }
            Err(error) => println!("track {track_id} in {named}: not read: {error}"),
        }
    }
    Ok(())
}

/// Gathers the samples of `locations` out of the file at `path`, seeking to the bytes a media data reader wants, and returns how many there are and their bytes
fn gather(path: &Path, locations: Vec<SampleLocation>) -> Result<(usize, usize), Box<dyn Error>> {
    let mut file = BufferedFile::open(path)?;
    let mut media_data = MediaDataReader::new(DemuxLimits::new().media_data());
    media_data.handle_sample_locations(locations)?;
    let (mut samples, mut bytes) = (0_usize, 0_usize);
    while let Some(wanted) = media_data.wanted_input() {
        let (buffered_at, input) = file.read_at(wanted.offset())?;
        if input.is_empty() {
            break;
        }
        let handled = media_data.handle_input(buffered_at, input);
        while let Some(sample) = media_data.poll_sample() {
            samples = samples.saturating_add(1);
            bytes = bytes.saturating_add(sample.data().len());
        }
        handled?;
    }
    media_data.finish()?;
    Ok((samples, bytes))
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
