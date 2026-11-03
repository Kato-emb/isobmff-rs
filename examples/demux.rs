//! Lists the boxes and the samples of a movie file or a segment, seeking to the bytes the demuxer wants
//!
//! ```text
//! cargo run --example demux -- movie.mp4
//! ```
//!
//! The demuxer names the bytes it wants next. The caller reads the file
//! through a `BufReader` and moves to that file offset with `seek_relative`,
//! which keeps the buffer when the offset lies within it. It passes the
//! buffered bytes from there, which `fill_buf` returns, for as long as the
//! demuxer returns events, and reads more only once the demuxer wants bytes
//! past them. No byte from `fill_buf` is the end of the file, which the
//! caller tells the demuxer with `finish`.
//!
//! The demuxer skips each `mdat`, and once a `moov` or a `moof` locates
//! samples in it, wants their bytes. The samples come out of `poll_sample`
//! once their bytes have arrived, those whose bytes came with a call that
//! fails among them: the caller takes them before it returns the failure.
//!
//! The media data of a track may be in another file, which an entry of its
//! data reference names (ISO/IEC 14496-12 §8.7.2). The demuxer locates those
//! samples and does not gather them, and this example does not read them
//! either: it counts them by data reference entry and names the file each is
//! in. The crate documentation shows how a caller gathers them.

use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek};

use isobmff::boxes::DataEntry;
use isobmff::{ByteRange, DataReferenceKey, DemuxEvent, Demuxer, Sample};

/// Samples listed per track
const LISTED_SAMPLES: usize = 3;

fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args().nth(1).ok_or("usage: demux <file>")?;
    let mut file = BufReader::with_capacity(64 * 1024, File::open(path)?);
    let mut demuxer = Demuxer::new();
    let mut samples = BTreeMap::<u32, Vec<Sample>>::new();
    let mut elsewhere = BTreeMap::<DataReferenceKey, usize>::new();

    while let Some(wanted) = demuxer.wanted_input() {
        let position = file.stream_position()?;
        let distance = i64::try_from(wanted.offset())?
            .checked_sub(i64::try_from(position)?)
            .ok_or("an offset past 63 bits")?;
        file.seek_relative(distance)?;
        let event = match file.fill_buf()? {
            [] => demuxer.finish(),
            input => demuxer.handle_input(wanted.offset(), input),
        };
        while let Some(sample) = demuxer.poll_sample() {
            samples
                .entry(sample.properties().track_id)
                .or_default()
                .push(sample);
        }
        if let Some(event) = event.inspect_err(|_| list_samples(&samples))? {
            if let DemuxEvent::Movie { samples, .. } | DemuxEvent::MovieFragment { samples, .. } =
                &event
            {
                for location in samples.clone() {
                    let key = location.data_reference();
                    if demuxer
                        .data_reference(key)
                        .is_some_and(|data_reference| !data_reference.entry().is_in_same_file())
                    {
                        let count = elsewhere.entry(key).or_default();
                        *count = count.saturating_add(1);
                    }
                }
            }
            report(&demuxer, event)?;
        }
    }

    list_samples(&samples);
    for (key, count) in elsewhere {
        let Some(data_reference) = demuxer.data_reference(key) else {
            continue;
        };
        let entry = data_reference.entry();
        let named = if let DataEntry::Url(url) = entry
            && let Some(location) = url.location.as_ref().and_then(|location| location.as_str())
        {
            location.to_owned()
        } else {
            format!("{entry:?}")
        };
        println!(
            "track {}: {count} samples in {named}, not read",
            data_reference.track_id()
        );
    }
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
