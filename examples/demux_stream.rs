//! Lists the boxes and the samples of a movie file or a segment as its bytes arrive over HTTP, without seeking
//!
//! ```text
//! cargo run --example demux_stream -- https://example.com/movie.mp4
//! ```
//!
//! The response body is read a buffer at a time. Each buffer goes
//! to the demuxer again and again while it returns events, one box at file
//! level each. After each call, the samples whose bytes have arrived by then
//! come out of `poll_sample`, in the order of their locations; those of a
//! call that fails are taken before the failure is returned. Each box and
//! each sample is printed as it comes out.
//!
//! The demuxer cannot tell where the file ends, so the caller calls `finish`
//! when the response body ends.
//!
//! A caller that cannot seek reads the samples of a file whose `moov` and
//! `moof`s each come before their media data: media data in a buffer passed
//! before the box that locates it is read is not kept, and `finish` fails.
//!
//! The demuxer holds the media data of the samples not yet taken up to
//! `DemuxLimits::with_held_bytes`, so the samples are taken after each call,
//! and a buffer is passed again while the demuxer wants bytes of it: those of
//! a sample it could hold only once the samples before it were taken.

use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::io::Read;

use isobmff::boxes::{
    FileTypeBox, MovieBox, MovieFragmentBox, MovieFragmentRandomAccessBox, SegmentIndexBox,
    SegmentTypeBox,
};
use isobmff::{BoxDefinition, DemuxEvent, Demuxer};
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
    let mut gathered = BTreeMap::<u32, (u64, u64)>::new();

    loop {
        let read = body.read(&mut buffer)?;
        let piece = buffer.get(..read).ok_or("read past the buffer")?;
        if piece.is_empty() {
            break;
        }
        let end = offset.saturating_add(u64::try_from(read)?);
        loop {
            let event = demuxer.handle_input(offset, piece);
            if let Ok(Some(event)) = &event {
                print_box(event);
            }
            take_samples(&mut demuxer, &mut gathered)?;
            if event?.is_none()
                && !demuxer
                    .wanted_input()
                    .is_some_and(|wanted| (offset..end).contains(&wanted.offset()))
            {
                break;
            }
        }
        offset = end;
    }
    loop {
        let event = demuxer.finish(offset);
        if let Ok(Some(event)) = &event {
            print_box(event);
        }
        take_samples(&mut demuxer, &mut gathered)?;
        if event?.is_none() {
            break;
        }
    }

    for (track_id, (samples, bytes)) in gathered {
        println!("track {track_id}: {samples} samples, {bytes} bytes");
    }
    for leniency in demuxer.leniencies() {
        println!("read past: {leniency}");
    }
    Ok(())
}

/// Prints one line for the box at file level `event` reports: its type and where it lies
fn print_box(event: &DemuxEvent) {
    let (box_type, byte_range) = match event {
        DemuxEvent::FileType { byte_range, .. } => (FileTypeBox::BOX_TYPE, byte_range),
        DemuxEvent::SegmentType { byte_range, .. } => (SegmentTypeBox::BOX_TYPE, byte_range),
        DemuxEvent::Movie { byte_range, .. } => (MovieBox::BOX_TYPE, byte_range),
        DemuxEvent::MovieFragment { byte_range, .. } => (MovieFragmentBox::BOX_TYPE, byte_range),
        DemuxEvent::SegmentIndex { byte_range, .. } => (SegmentIndexBox::BOX_TYPE, byte_range),
        DemuxEvent::MovieFragmentRandomAccess { byte_range, .. } => {
            (MovieFragmentRandomAccessBox::BOX_TYPE, byte_range)
        }
        DemuxEvent::MediaData {
            header, byte_range, ..
        }
        | DemuxEvent::Other {
            header, byte_range, ..
        } => (header.box_type(), byte_range),
        _ => return,
    };
    let size = byte_range
        .length()
        .map_or_else(|| "to end of file".to_owned(), |length| length.to_string());
    println!("[{box_type}] offset={} size={size}", byte_range.offset());
}

/// Prints each sample `demuxer` has gathered, and counts them and their bytes by track
fn take_samples(
    demuxer: &mut Demuxer,
    gathered: &mut BTreeMap<u32, (u64, u64)>,
) -> Result<(), Box<dyn Error>> {
    while let Some(sample) = demuxer.poll_sample() {
        let properties = sample.properties();
        println!(
            "  sample of track {}: decoding time {}, {} bytes",
            properties.track_id,
            properties.decoding_time,
            sample.data().len()
        );
        let (samples, bytes) = gathered.entry(properties.track_id).or_default();
        *samples = samples.saturating_add(1);
        *bytes = bytes.saturating_add(u64::try_from(sample.data().len())?);
    }
    Ok(())
}
