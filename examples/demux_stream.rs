//! Lists the boxes and the samples of a movie file or a segment as a demuxer reads them, as its bytes arrive over HTTP
//!
//! ```text
//! cargo run --example demux_stream -- https://example.com/movie.mp4
//! ```
//!
//! The caller passes the response body to the demuxer in the order it
//! arrives, without seeking. For each box at file level, the demuxer returns
//! an event. It returns each sample once the bytes of the sample have
//! arrived, and the caller takes the samples after each call, before it
//! looks at the event or the failure. The caller passes a piece again while
//! the demuxer wants bytes inside it, and tells the demuxer with `finish`
//! when the body ends. Media data that arrives before the box that locates it
//! is not kept, so the samples of a file whose `moov` comes after its media
//! data are not returned, and `finish` fails.
//!
//! The media data of a track may be in another file, which an entry of its
//! data reference names (ISO/IEC 14496-12 §8.7.2). The demuxer locates those
//! samples but does not gather them. This example counts them and names the
//! files they are in.

use std::env;
use std::error::Error;
use std::io::Read;

use isobmff::boxes::DataEntry;
use isobmff::{DemuxEvent, Demuxer};
use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig};

fn main() -> Result<(), Box<dyn Error>> {
    let url = env::args().nth(1).ok_or("usage: demux_stream <url>")?;
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| "a crypto provider is already installed")?;
    let agent: Agent = Agent::config_builder()
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into();
    let mut body = agent.get(&url).call()?.into_body().into_reader();
    let mut demuxer = Demuxer::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut offset = 0_u64;

    loop {
        let read = body.read(&mut buffer)?;
        let piece = buffer.get(..read).ok_or("read past the buffer")?;
        if piece.is_empty() {
            break;
        }
        let end = offset.saturating_add(u64::try_from(read)?);
        loop {
            let event = demuxer.handle_input(offset, piece);
            print_samples(&mut demuxer);
            let Some(event) = event? else {
                if demuxer
                    .wanted_input()
                    .is_some_and(|wanted| (offset..end).contains(&wanted.offset()))
                {
                    continue;
                }
                break;
            };
            print_event(&demuxer, &event);
        }
        offset = end;
    }
    loop {
        let event = demuxer.finish();
        print_samples(&mut demuxer);
        let Some(event) = event? else {
            break;
        };
        print_event(&demuxer, &event);
    }

    for data_reference in demuxer.data_references() {
        let entry = data_reference.entry();
        if entry.is_in_same_file() {
            continue;
        }
        let named = if let DataEntry::Url(url) = entry
            && let Some(location) = url.location.as_ref().and_then(|location| location.as_str())
        {
            location.to_owned()
        } else {
            format!("{entry:?}")
        };
        println!(
            "track {}: samples in {named}, not read",
            data_reference.track_id()
        );
    }
    Ok(())
}

/// Prints one line for each sample `demuxer` has gathered
fn print_samples(demuxer: &mut Demuxer) {
    while let Some(sample) = demuxer.poll_sample() {
        let properties = sample.properties();
        println!(
            "  sample of track {}: decoding time {}, {} bytes",
            properties.track_id,
            properties.decoding_time,
            sample.data().len()
        );
    }
}

/// Prints one line for the box at file level `event` reports, and one more for the samples a `moov` or a `moof` locates
fn print_event(demuxer: &Demuxer, event: &DemuxEvent) {
    let (box_type, byte_range) = match event {
        DemuxEvent::FileType { byte_range, .. } => ("ftyp".to_owned(), byte_range),
        DemuxEvent::SegmentType { byte_range, .. } => ("styp".to_owned(), byte_range),
        DemuxEvent::Movie { byte_range, .. } => ("moov".to_owned(), byte_range),
        DemuxEvent::MovieFragment { byte_range, .. } => ("moof".to_owned(), byte_range),
        DemuxEvent::SegmentIndex { byte_range, .. } => ("sidx".to_owned(), byte_range),
        DemuxEvent::MovieFragmentRandomAccess { byte_range, .. } => ("mfra".to_owned(), byte_range),
        DemuxEvent::MediaData {
            header, byte_range, ..
        }
        | DemuxEvent::Other {
            header, byte_range, ..
        } => (header.box_type().to_string(), byte_range),
        _ => return,
    };
    let size = byte_range
        .length()
        .map_or_else(|| "to end of file".to_owned(), |length| length.to_string());
    println!("[{box_type}] offset={} size={size}", byte_range.offset());

    if let DemuxEvent::Movie { samples, .. } | DemuxEvent::MovieFragment { samples, .. } = event {
        let elsewhere = samples
            .clone()
            .filter(|location| {
                demuxer
                    .data_reference(location.data_reference())
                    .is_some_and(|data_reference| !data_reference.entry().is_in_same_file())
            })
            .count();
        println!(
            "  locates {} samples, {elsewhere} of them in other files",
            samples.len()
        );
    }
}
