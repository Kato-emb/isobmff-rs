//! Sans-IO reading and writing of ISO base media files (ISO/IEC 14496-12)
//!
//! A [`Demuxer`] reads a movie file or a segment from the bytes a caller
//! reads. It does no I/O: it names the bytes it wants next, takes them with
//! the file offset they start at, and reports what they hold. It reads the
//! structure-data and skips the boxes of media data, then wants the bytes of
//! the samples the structure-data locates, and returns each sample once they
//! have arrived. A caller that can seek reads what the demuxer wants:
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
//!         [] => demuxer.finish(),
//!         input => demuxer.handle_input(wanted.offset(), input),
//!     };
//!     // The samples whose bytes have arrived, before a failure of the call is returned
//!     samples.extend(core::iter::from_fn(|| demuxer.poll_sample()));
//!     if let Some(DemuxEvent::Movie { samples: locations, .. }) = event? {
//!         assert_eq!(locations.map(|location| location.byte_range()).collect::<Vec<_>>(), [497..499, 499..501]);
//!     }
//! }
//! assert_eq!(samples.iter().map(|sample| sample.data()).collect::<Vec<_>>(), [b"AB", b"CD"]);
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! A caller that only indexes a file, or plays some of its tracks,
//! [discards](Demuxer::discard_track) the others: their samples are not
//! gathered, and their bytes are not wanted. The events carry the location of
//! every sample.
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
//! A caller that cannot seek passes the file to the demuxer in order, a piece
//! at a time, passing each piece again while the demuxer returns events:
//!
//! ```
//! use isobmff::Demuxer;
//!
//! let file: &[u8] = include_bytes!("../tests/data/movie.mp4");
//! let (mut demuxer, mut samples) = (Demuxer::new(), Vec::new());
//!
//! let mut offset = 0_u64;
//! for piece in file.chunks(64) {
//!     // The same piece again while events come, taking the samples after each call
//!     loop {
//!         let event = demuxer.handle_input(offset, piece);
//!         samples.extend(core::iter::from_fn(|| demuxer.poll_sample()));
//!         if event?.is_none() { break; }
//!     }
//!     offset += u64::try_from(piece.len())?;
//! }
//! // The input has ended, which only the caller knows
//! loop {
//!     let event = demuxer.finish();
//!     samples.extend(core::iter::from_fn(|| demuxer.poll_sample()));
//!     if event?.is_none() { break; }
//! }
//! assert_eq!(samples.iter().map(|sample| sample.data()).collect::<Vec<_>>(), [b"AB", b"CD"]);
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! Such a caller reads the samples of a file whose `moov` and `moof`s each
//! come before their media data: media data in a piece passed before the box
//! that locates it is read is not kept, and [`finish`](Demuxer::finish) fails
//! with [`UnfinishedSample`](DemuxError::UnfinishedSample). The demuxer holds
//! the locations of every sample of a `moov` or a `moof` at once, so their
//! bytes together have to fit [`DemuxLimits::with_held_bytes`].
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
//! /// Reads `file` up to the first event `found` accepts
//! fn read_to(demuxer: &mut Demuxer, file: &[u8], found: fn(&DemuxEvent) -> bool)
//!     -> Result<DemuxEvent, Box<dyn Error>> {
//!     while let Some(wanted) = demuxer.wanted_input() {
//!         let input = file.get(usize::try_from(wanted.offset())?..).filter(|input| !input.is_empty());
//!         if let Some(event) = demuxer.handle_input(wanted.offset(), input.ok_or("not found")?)? {
//!             if found(&event) { return Ok(event); }
//!         }
//!     }
//!     Err("not found".into())
//! }
//!
//! let file: &[u8] = include_bytes!("../tests/data/fragmented.mp4");
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
//! The examples in `crates/isobmff/examples/` show the loops whole.
//! `cargo run -p isobmff --example demux -- movie.mp4` seeks through a file
//! to what the demuxer wants, and `demux_stream` reads standard input in
//! order; both count the samples the demuxer returns. `dump_boxes` and
//! `split_fragments` read and write files through [`framing`]: one lists the
//! boxes of a file, the other splits a fragmented movie file into segments.

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
