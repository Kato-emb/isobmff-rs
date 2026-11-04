//! Lists where on the presentation timeline each sample of a movie file or a segment is presented, as the edit list of its track maps it
//!
//! ```text
//! cargo run --example presentation_times -- movie.mp4
//! ```
//!
//! The demuxer locates each sample at its decoding time. The edit list of
//! its track (ISO/IEC 14496-12 §8.6.6) decides where it is presented: later,
//! after an empty edit, not at all, or more than once. A `Timeline` maps
//! each sample to its presentations. The times are in the timescale of the
//! media of the track, as the samples' are. The example lists the samples
//! the `moov` and the `moof`s locate, in the order their media data lies,
//! those in other files too, and does not read their media data.

use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};

use isobmff::timeline::Timeline;
use isobmff_mux::{DemuxEvent, Demuxer};

/// Presentations of a sample listed, past which a repeated edit list is cut short
const LISTED: usize = 4;

fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args()
        .nth(1)
        .ok_or("usage: presentation_times <file>")?;
    let mut file = BufReader::new(File::open(path)?);
    let mut demuxer = Demuxer::new();

    // The demuxer wants no input past a box that runs to the end of the file, which ends only once the file is finished.
    loop {
        while let Some(wanted) = demuxer.wanted_input() {
            file.seek(SeekFrom::Start(wanted.offset()))?;
            let event = match file.fill_buf()? {
                [] => demuxer.finish(),
                input => demuxer.handle_input(wanted.offset(), input),
            };
            if let Some(event) = event? {
                list(&mut demuxer, event)?;
            }
        }
        let Some(event) = demuxer.finish()? else {
            break;
        };
        list(&mut demuxer, event)?;
    }
    Ok(())
}

/// Lists the presentations of the samples a `moov` or a `moof` `event` locates
fn list(demuxer: &mut Demuxer, event: DemuxEvent) -> Result<(), Box<dyn Error>> {
    let (DemuxEvent::Movie { samples, .. } | DemuxEvent::MovieFragment { samples, .. }) = event
    else {
        return Ok(());
    };
    // Why not drain poll_sample: the demuxer would want and gather the media data first, which the listing never reads.
    let track_ids: Vec<u32> = demuxer
        .movie()
        .iter()
        .flat_map(|movie| &movie.trak)
        .map(|trak| trak.tkhd.track_id)
        .collect();
    for track_id in track_ids {
        demuxer.discard_track(track_id);
    }
    let movie = demuxer.movie().ok_or("samples before the movie")?;
    for location in samples {
        let properties = location.properties();
        let (track_id, decoding_time) = (properties.track_id, properties.decoding_time);
        let presentations: Vec<_> = Timeline::new(movie, track_id)?
            .present(properties)
            .take(LISTED + 1)
            .collect();
        if presentations.is_empty() {
            println!("track {track_id}: decoding {decoding_time}, not presented");
        }
        for presentation in presentations.iter().take(LISTED) {
            println!(
                "track {track_id}: decoding {decoding_time} -> presented at {} for {}",
                presentation.presentation_time(),
                presentation.duration()
            );
        }
        if presentations.len() > LISTED {
            println!(
                "track {track_id}: decoding {decoding_time}, presented again as its edit list repeats"
            );
        }
    }
    Ok(())
}
