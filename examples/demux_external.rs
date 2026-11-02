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
//! It keeps no more of them, over all the movie fragments, than a media
//! data reader holds (`MediaDataLimits::held_location_count`), and fails
//! past that.
//!
//! Once it is read, the caller opens the file the entry of each key names,
//! taking its location as a path relative to the directory of the movie
//! file, and gathers the samples in it with a `MediaDataReader` of its own,
//! seeking to the bytes the reader wants as it does for the demuxer. A
//! location with a scheme (`file:`, `http:`) or an absolute path, an entry
//! other than a `url `, and a file that does not open are reported, and
//! their samples are not read.

use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek};
use std::path::{Path, PathBuf};

use isobmff::boxes::DataEntry;
use isobmff::media_data::{MediaDataLimits, MediaDataReader};
use isobmff::{DataReferenceKey, DemuxEvent, DemuxLimits, Demuxer, SampleLocation};

fn main() -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(
        env::args_os()
            .nth(1)
            .ok_or("usage: demux_external <file>")?,
    );
    let mut file = BufReader::with_capacity(64 * 1024, File::open(&path)?);
    let mut demuxer = Demuxer::new();
    let (mut samples_here, mut bytes_here) = (0_usize, 0_usize);
    let limits = DemuxLimits::new().media_data();
    let mut by_key = BTreeMap::<DataReferenceKey, Vec<SampleLocation>>::new();
    let mut held = 0_u64;

    while let Some(wanted) = demuxer.wanted_input() {
        let event = match read_at(&mut file, wanted.offset())? {
            [] => demuxer.finish(),
            input => demuxer.handle_input(wanted.offset(), input),
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
                held = held.saturating_add(1);
                if held > limits.held_location_count {
                    return Err(format!(
                        "more than {} samples in other files",
                        limits.held_location_count
                    )
                    .into());
                }
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
        let Some(named) =
            named.filter(|named| Path::new(named).is_relative() && !named.contains(':'))
        else {
            println!(
                "track {track_id}: {} samples in {:?}, not read",
                locations.len(),
                data_reference.entry()
            );
            continue;
        };
        match gather(&path.with_file_name(named), locations, limits) {
            Ok((samples, bytes)) => {
                println!("track {track_id} in {named}: {samples} samples, {bytes} bytes");
            }
            Err(error) => println!("track {track_id} in {named}: not read: {error}"),
        }
    }
    Ok(())
}

/// Gathers the samples of `locations` out of the file at `path` with a media data reader of `limits`, seeking to the bytes it wants, and returns how many there are and their bytes
fn gather(
    path: &Path,
    locations: Vec<SampleLocation>,
    limits: MediaDataLimits,
) -> Result<(usize, usize), Box<dyn Error>> {
    let mut file = BufReader::with_capacity(64 * 1024, File::open(path)?);
    let mut media_data = MediaDataReader::new(limits);
    media_data.handle_sample_locations(locations)?;
    let (mut samples, mut bytes) = (0_usize, 0_usize);
    while let Some(wanted) = media_data.wanted_input() {
        let input = read_at(&mut file, wanted.offset())?;
        if input.is_empty() {
            break;
        }
        let handled = media_data.handle_input(wanted.offset(), input);
        while let Some(sample) = media_data.poll_sample() {
            samples = samples.saturating_add(1);
            bytes = bytes.saturating_add(sample.data().len());
        }
        handled?;
    }
    media_data.finish()?;
    Ok((samples, bytes))
}

/// Returns the bytes `file` buffers from file offset `offset` on, reading more where it buffers none there; none at the end of the file
fn read_at(file: &mut BufReader<File>, offset: u64) -> Result<&[u8], Box<dyn Error>> {
    let position = file.stream_position()?;
    let distance = i64::try_from(offset)?
        .checked_sub(i64::try_from(position)?)
        .ok_or("an offset past 63 bits")?;
    file.seek_relative(distance)?;
    Ok(file.fill_buf()?)
}
