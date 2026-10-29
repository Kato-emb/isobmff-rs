//! Sans-IO reading and writing of ISO base media files (ISO/IEC 14496-12)
//!
//! A [`Demuxer`] reads a movie file or a segment from the bytes a caller
//! reads. It does no I/O: it names the bytes it wants next, takes them with
//! the file offset they start at, and reports what they hold. A caller that
//! can seek reads what the demuxer wants, skips the media data, and reads the
//! byte range of each sample itself:
//!
//! ```
//! use isobmff::{DemuxEvent, Demuxer};
//!
//! let file: &[u8] = include_bytes!("../tests/data/movie.mp4");
//! let (mut demuxer, mut samples) = (Demuxer::new(), Vec::new());
//!
//! // Seek to the offset the demuxer wants, and read from there
//! while let Some(wanted) = demuxer.wanted_input() {
//!     let input = file.get(usize::try_from(wanted.offset())?..).unwrap_or_default();
//!     // Past the last byte, the file ends, and so may a box that runs to its end
//!     let event = match input {
//!         [] => demuxer.finish()?,
//!         input => demuxer.handle_input(wanted.offset(), input)?,
//!     };
//!     match event {
//!         Some(DemuxEvent::Movie { samples: locations, .. }) => samples.extend(locations),
//!         Some(DemuxEvent::MediaData { .. }) => demuxer.skip()?,
//!         _ => {}
//!     }
//! }
//! assert_eq!(samples.len(), 2);
//! assert_eq!(samples[1].byte_range(), 499..501);
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! The demuxer is made of layers that follow the structure of a file, each
//! usable alone:
//!
//! | Layer | Reads | ISO/IEC 14496-12 |
//! |---|---|---|
//! | [`framing`] | the boxes laid end to end, out of bytes that arrive in pieces | §4.2.2 |
//! | [`structure_data`] | the `moov` and `moof`s, resolved to the [`SampleLocation`] of each sample | A.3, §8.5, §8.8 |
//! | [`media_data`] | the bytes of each sample, gathered out of the bytes of the file | §8.1.1 |
//!
//! The demuxer composes the first two. A caller that cannot seek streams the
//! media data instead, and gathers its samples with a
//! [`MediaDataReader`](media_data::MediaDataReader):
//!
//! ```
//! use isobmff::media_data::MediaDataReader;
//! use isobmff::{DemuxEvent, DemuxLimits, Demuxer};
//!
//! let file: &[u8] = include_bytes!("../tests/data/movie.mp4");
//! let mut demuxer = Demuxer::new();
//! let mut media_data = MediaDataReader::new(DemuxLimits::new().media_data());
//! let mut samples = Vec::new();
//!
//! // The file arrives in order, a piece at a time, then `None`: the caller knows it has ended
//! for (offset, piece) in (0_u64..).step_by(64).zip(file.chunks(64).map(Some).chain([None])) {
//!     let end = offset + u64::try_from(piece.map_or(0, <[u8]>::len))?;
//!     // A piece is passed until the demuxer wants past it
//!     while demuxer.wanted_input().is_some_and(|wanted| piece.is_none() || wanted.offset() < end) {
//!         let event = match piece {
//!             Some(piece) => demuxer.handle_input(offset, piece)?,
//!             None => demuxer.finish()?,
//!         };
//!         match event {
//!             Some(DemuxEvent::Movie { samples, .. } | DemuxEvent::MovieFragment { samples, .. }) =>
//!                 media_data.handle_sample_locations(samples)?,
//!             Some(DemuxEvent::MediaData { .. }) => demuxer.stream()?,
//!             Some(DemuxEvent::MediaDataPayload { offset, bytes, .. }) => media_data.handle_input(offset, bytes),
//!             _ => {}
//!         }
//!         samples.extend(core::iter::from_fn(|| media_data.poll_sample()));
//!     }
//! }
//! media_data.finish()?;
//! assert_eq!(samples.iter().map(|sample| sample.data()).collect::<Vec<_>>(), [b"AB", b"CD"]);
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! An input that ends before the offset the demuxer wants holds nothing for it,
//! so such a caller calls [`finish`](Demuxer::finish) itself when its input
//! ends. It reads the samples of a fragmented movie file, or of a file whose
//! `moov` comes before its media data: media data that goes by before the
//! `moov` is not kept, and
//! [`MediaDataReader::finish`](media_data::MediaDataReader::finish) fails with
//! [`UnfinishedSample`](DemuxError::UnfinishedSample). The locations a
//! `MediaDataReader` holds count toward its limit by the bytes they name, so a
//! caller that hands it every sample of a `moov` or a `moof` at once, as here,
//! reads only one whose samples together fit the limit.
//!
//! A caller that can seek reads a fragmented movie file from any of its
//! fragments. The `mfra` at the end of the file lists where they start
//! (§8.8.9), and a fragment with a `tfdt` gives its own decoding time:
//!
//! ```
//! use isobmff::structure_data::movie_fragment_random_access_start;
//! use isobmff::{DemuxEvent, Demuxer};
//! # use core::error::Error;
//!
//! /// Reads `file`, skipping the media data, up to the first event `found` accepts
//! fn read_to(demuxer: &mut Demuxer, file: &'static [u8], found: fn(&DemuxEvent<'_>) -> bool)
//!     -> Result<DemuxEvent<'static>, Box<dyn Error>> {
//!     while let Some(wanted) = demuxer.wanted_input() {
//!         let input = file.get(usize::try_from(wanted.offset())?..).filter(|input| !input.is_empty());
//!         if let Some(event) = demuxer.handle_input(wanted.offset(), input.ok_or("not found")?)? {
//!             if let DemuxEvent::MediaData { .. } = event { demuxer.skip()?; }
//!             if found(&event) { return Ok(event); }
//!         }
//!     }
//!     Err("not found".into())
//! }
//!
//! let file: &'static [u8] = include_bytes!("../tests/data/fragmented.mp4");
//! let mut demuxer = Demuxer::new();
//! read_to(&mut demuxer, file, |event| matches!(event, DemuxEvent::Movie { .. }))?;
//! // The `mfro` in the last 16 bytes gives where the `mfra` starts
//! let mfra = movie_fragment_random_access_start(u64::try_from(file.len())?, file.last_chunk().ok_or("short")?);
//! demuxer.resume_at(mfra.ok_or("no mfra")?)?;
//! read_to(&mut demuxer, file, |event| matches!(event, DemuxEvent::MovieFragmentRandomAccess { .. }))?;
//! // Resume at the last fragment the `mfra` lists
//! let tfra = demuxer.movie_fragment_random_access().and_then(|mfra| mfra.tfra.first());
//! let last = tfra.and_then(|tfra| tfra.entries.last()).ok_or("no fragment")?.clone();
//! demuxer.resume_at(last.moof_offset)?;
//! let found = read_to(&mut demuxer, file, |event| matches!(event, DemuxEvent::MovieFragment { .. }))?;
//! let DemuxEvent::MovieFragment { samples, .. } = found else { unreachable!() };
//! let decoding_times: Vec<_> = samples.map(|sample| sample.properties().decoding_time).collect();
//! assert_eq!(decoding_times, [last.time, last.time + 100]);
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! The items of [`isobmff_core`], the vocabulary a box is written in, are at
//! the root of this crate. The boxes ISO/IEC 14496-12 defines, written in that
//! vocabulary, are in [`boxes`].
//!
//! The examples in `crates/isobmff/examples/` read and write files through
//! [`framing`]: `cargo run -p isobmff --example dump_boxes -- movie.mp4` lists
//! the boxes of a file, and `split_fragments` splits a fragmented movie file
//! into segments.

#![no_std]

extern crate alloc;

mod byte_range;
mod demuxer;
mod error;
pub mod framing;
mod limits;
pub mod media_data;
mod sample;
pub mod structure_data;

pub use byte_range::ByteRange;
pub use demuxer::{DemuxEvent, Demuxer};
pub use error::{DemuxError, MuxError};
pub use isobmff_core::*;
pub use limits::DemuxLimits;
pub use sample::{Sample, SampleLocation, SampleProperties};

/// Boxes of ISO/IEC 14496-12, each read into a value that owns its bytes
///
/// These are the items of [`isobmff_boxes`].
pub mod boxes {
    pub use isobmff_boxes::*;
}
