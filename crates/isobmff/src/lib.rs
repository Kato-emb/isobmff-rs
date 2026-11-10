//! Sans-IO reading and writing of ISO base media files (ISO/IEC 14496-12)
//!
//! [`Demuxer`] reads a file into its `ftyp` and `moov`, its movie fragments,
//! the location of each sample, and the bytes of the samples of the tracks a
//! caller subscribes to.
//! It does no I/O: it names where it wants input next, and the caller reads
//! the file however it can. The [`demuxer`] module holds the events it
//! reports.
//!
//! ```
//! use isobmff::demuxer::Event;
//! use isobmff::{Demuxer, DemuxerConfig};
//!
//! let file: &[u8] = include_bytes!("../tests/fixtures/movie_moov_first.mp4");
//! let mut demuxer = Demuxer::new(DemuxerConfig::default());
//! let (mut locations, mut samples) = (0, Vec::new());
//! loop {
//!     // Report what the bytes handed over hold, subscribing to every track
//!     while let Some(event) = demuxer.poll_event() {
//!         match event {
//!             Event::Movie(movie) => {
//!                 for trak in &movie.trak {
//!                     demuxer.subscribe_track(trak.tkhd.track_id);
//!                 }
//!             }
//!             Event::SampleLocation(_) => locations += 1,
//!             Event::Sample(sample) => samples.push((sample.properties.track_id, sample.bytes.len())),
//!             Event::Error(error) => return Err(error.into()),
//!             _ => {}
//!         }
//!     }
//!     // Read the file where the demuxer asks, and end it where it ends
//!     let Some(required) = demuxer.required_input() else { break };
//!     match file.get(usize::try_from(required.position)?..) {
//!         Some(bytes) if !bytes.is_empty() => demuxer.handle_input(required.position, bytes),
//!         _ => demuxer.finish(),
//!     }
//! }
//! assert_eq!((locations, samples.len()), (12 + 58, 12 + 58));
//! assert_eq!(samples.iter().filter(|(track_id, _)| *track_id == 1).count(), 12);
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! [`MovieFileMuxer`] writes a movie file, its `ftyp`, one `mdat` and the
//! `moov` that declares it, from the samples a caller hands it; the
//! [`muxer`] module holds what describes the movie and its tracks.
//!
//! The layers they are made of are usable alone: [`framing`] reads the boxes
//! laid end to end in a file from bytes that arrive in pieces, and
//! [`structure_data`] resolves the samples a `moov` or a `moof` declares to where their
//! bytes lie, and writes the sample table of a track from them. The boxes
//! themselves are read and written with `isobmff-boxes`.

#![no_std]

extern crate alloc;

pub mod demuxer;
pub mod framing;
mod input_buffer;
pub mod muxer;
mod sample;
pub mod structure_data;

pub use demuxer::{Demuxer, DemuxerConfig};
pub use muxer::{MovieFileMuxer, MuxerConfig};
pub use sample::{SampleLocation, SampleProperties};

/// Where a reader wants input next
///
/// The reader takes bytes from any file offset up to `position`, and ignores
/// those before it.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequiredInput {
    /// File offset of the first byte the reader wants
    pub position: u64,
    /// How many bytes from `position` the reader needs at least before it
    /// reports what they hold, when it knows
    pub length: Option<u64>,
}
