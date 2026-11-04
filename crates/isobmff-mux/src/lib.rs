//! Sans-IO demuxer, muxer and fast start of ISO base media files (ISO/IEC 14496-12), made of the layers of `isobmff`
//!
//! A [`Muxer`] writes the samples a caller hands it into a movie file or a
//! segment, and a [`FastStart`] lays a movie file out again with its `moov`
//! before its media data; each returns the file a box at file level at a
//! time, with the file offset it goes to. The values these take and return,
//! the boxes, the samples and their locations, are those of [`isobmff`],
//! which this crate does not repeat.
//!
//! A [`Demuxer`] reads a movie file or a segment from the bytes a caller
//! reads. It does no I/O: it names the bytes it wants next, takes them with
//! the file offset they start at, and reports what they hold. It reads the
//! structure-data and skips the boxes of media data, then wants the bytes of
//! the samples the structure-data locates, and returns each sample once they
//! have arrived. A caller that can seek reads what the demuxer wants:
//!
//! ```
//! use isobmff_mux::{DemuxEvent, Demuxer};
//!
//! let file: &[u8] = include_bytes!("../../isobmff/tests/data/movie.mp4");
//! let (mut demuxer, mut samples) = (Demuxer::new(), Vec::new());
//!
//! // Seek to the offset the demuxer wants, and read from there
//! loop {
//!     let event = match demuxer.wanted_input() {
//!         Some(wanted) => {
//!             let input = file.get(usize::try_from(wanted.offset())?..).unwrap_or_default();
//!             // Past the last byte, the file ends, and so may a box that runs to its end
//!             match input {
//!                 [] => demuxer.finish(),
//!                 input => demuxer.handle_input(wanted.offset(), input),
//!             }
//!         }
//!         // Nothing is wanted past a box that runs to the end of the file: it ends with the file
//!         None => match demuxer.finish() {
//!             Ok(None) => break,
//!             finished => finished,
//!         },
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
//! The samples come in the order their media data lies in the file, which
//! need not be the order of their decoding times (§8.7.5). A caller that can
//! seek takes the locations of a `moov` or a `moof`
//! [by track in decoding order](isobmff::structure_data::SampleLocations::into_decoding_order) and
//! reads the media data of each where it lies.
//!
//! A caller that only indexes a file, or plays some of its tracks,
//! [discards](Demuxer::discard_track) the others: their samples are not
//! gathered, and their bytes are not wanted. The events carry the location of
//! every sample.
//!
//! The media data of a track may be in an `IdentifiedMediaDataBox` (`imda`,
//! §8.1.4) of the file, which an entry of its data reference names by its
//! `imda_identifier` or by the sequence number of the movie fragment (§8.7.2).
//! The demuxer gathers those samples as it does the others: those an `imda`
//! read before holds come with their `moov` or `moof`, and those of an `imda`
//! read after it come with the
//! [`IdentifiedMediaData`](DemuxEvent::IdentifiedMediaData) event of the box.
//!
//! The media data of a track may be in another file, which an entry of its
//! data reference names (§8.7.2). The demuxer locates those samples in that
//! file and does not gather them. Each location carries the
//! [key](isobmff::SampleLocation::data_reference) of its entry, and
//! [`Demuxer::data_reference`] returns the entry of a key, from which the
//! caller reads that file and gathers them with a
//! [`MediaDataReader`](isobmff::media_data::MediaDataReader) of its own:
//!
//! ```
//! use std::collections::BTreeMap;
//!
//! use isobmff::boxes::DataEntry;
//! use isobmff::media_data::{MediaDataLimits, MediaDataReader};
//! use isobmff::{DataReferenceKey, SampleLocation};
//! use isobmff_mux::{DemuxEvent, Demuxer};
//!
//! let file: &[u8] = include_bytes!("../../isobmff/tests/data/external.mp4");
//! let mut demuxer = Demuxer::new();
//! let mut by_key = BTreeMap::<DataReferenceKey, Vec<SampleLocation>>::new();
//! loop {
//!     let event = match demuxer.wanted_input() {
//!         Some(wanted) => match file.get(usize::try_from(wanted.offset())?..).unwrap_or_default() {
//!             [] => demuxer.finish(),
//!             input => demuxer.handle_input(wanted.offset(), input),
//!         },
//!         None => match demuxer.finish() {
//!             Ok(None) => break,
//!             finished => finished,
//!         },
//!     }?;
//!     // Keep the locations by the key of their data reference entry
//!     if let Some(DemuxEvent::Movie { samples, .. }) = event {
//!         for location in samples {
//!             by_key.entry(location.data_reference()).or_default().push(location);
//!         }
//!     }
//!     assert_eq!(demuxer.poll_sample(), None);
//! }
//!
//! let mut samples = Vec::new();
//! for (key, locations) in by_key {
//!     // The demuxer gathers the samples in the file itself
//!     let entry = demuxer.data_reference(key).ok_or("no entry")?.entry();
//!     if entry.is_in_this_file() {
//!         continue;
//!     }
//!     // The entry names the file the byte ranges of the locations are in
//!     let DataEntry::Url(url) = entry else { return Err("not a url".into()) };
//!     let other: &[u8] = match url.location.as_ref().and_then(|location| location.as_str()) {
//!         Some("external.dat") => include_bytes!("../../isobmff/tests/data/external.dat"),
//!         _ => return Err("unknown file".into()),
//!     };
//!     // Read that file, and gather the samples out of it
//!     let mut media_data = MediaDataReader::new(MediaDataLimits {
//!         held_location_count: 1024,
//!         sample_bytes: 1024 * 1024,
//!         held_bytes: 16 * 1024 * 1024,
//!     });
//!     media_data.handle_sample_locations(locations)?;
//!     media_data.handle_input(0, other)?;
//!     media_data.finish()?;
//!     samples.extend(core::iter::from_fn(|| media_data.poll_sample()));
//! }
//! assert_eq!(samples.iter().map(|sample| sample.data()).collect::<Vec<_>>(), [b"AB", b"CD"]);
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! Which files to open is the caller's choice: §8.7.2 makes the location a
//! URL, which, where it is relative, is relative to the file that holds the
//! data reference. A caller that
//! reads local files commonly accepts only a relative path, resolved against
//! the directory of the movie file. A caller that keeps the locations of
//! samples in other files across movie fragments limits how many it keeps,
//! as [`MediaDataLimits::held_location_count`](isobmff::media_data::MediaDataLimits::held_location_count)
//! limits a media data reader; a location has a fixed size, so the count
//! limits their bytes too.
//!
//! A caller that cannot seek passes the file to the demuxer in order, a piece
//! at a time, passing each piece again while the demuxer returns events:
//!
//! ```
//! use isobmff_mux::Demuxer;
//!
//! let file: &[u8] = include_bytes!("../../isobmff/tests/data/movie.mp4");
//! let (mut demuxer, mut samples) = (Demuxer::new(), Vec::new());
//!
//! let mut offset = 0_u64;
//! for piece in file.chunks(64) {
//!     let end = offset + u64::try_from(piece.len())?;
//!     // The same piece again while events come, or while the demuxer wants bytes of it, taking the samples after each call
//!     loop {
//!         let event = demuxer.handle_input(offset, piece);
//!         samples.extend(core::iter::from_fn(|| demuxer.poll_sample()));
//!         if event?.is_none() && !demuxer.wanted_input().is_some_and(|wanted| (offset..end).contains(&wanted.offset())) {
//!             break;
//!         }
//!     }
//!     offset = end;
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
//! with [`UnfinishedSample`](isobmff::ReadError::UnfinishedSample).
//!
//! The demuxer allocates the media data of a sample when its first byte
//! arrives, and holds it until [`poll_sample`](Demuxer::poll_sample) takes the
//! sample. What it holds allocated is bounded by
//! [`DemuxLimits::with_held_bytes`]: the bytes past it are not kept and are
//! wanted again, so the loops above take the samples after each call, and the
//! caller that cannot seek passes a piece again while the demuxer wants bytes
//! of it. A caller that never takes them fails with
//! [`HeldBytesLimitExceeded`](isobmff::ReadError::HeldBytesLimitExceeded).
//!
//! Where the tables of a sample table state different numbers of samples,
//! the demuxer reads the file as conforming readers do rather than refuse
//! it, and keeps each such [`Leniency`](isobmff::Leniency) for the caller to ask after with
//! [`Demuxer::leniencies`].
//!
//! The samples carry their decoding times. Where a track has an edit list,
//! its samples are presented elsewhere on the presentation timeline, or
//! not at all, or more than once (§8.6.6): a [`Timeline`](isobmff::timeline::Timeline)
//! maps each sample to the times it is presented at.
//!
//! A caller that can seek reads a fragmented movie file from any of its
//! fragments. The `mfra` at the end of the file lists where they start
//! (§8.8.9), and a fragment with a `tfdt` gives its own decoding time:
//!
//! ```
//! use isobmff::structure_data::movie_fragment_random_access_start;
//! use isobmff_mux::{DemuxEvent, Demuxer};
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
//! let file: &[u8] = include_bytes!("../../isobmff/tests/data/fragmented.mp4");
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
//! The feature `compressed-boxes` reads the compressed boxes (ISO/IEC
//! 14496-12 §8.16), as the feature of the same name of [`isobmff`] does: the
//! demuxer reads a `!mov`, a `!mof` and a `!six` as the `moov`, `moof` and
//! `sidx` they replace, returns the samples they locate where their media
//! data lies in the file, takes the brands of a file from its `otyp`, and
//! refuses a file that has both a compressed box and an `mfra`. Without the
//! feature, the demuxer skips these boxes, as it does any box it reads no
//! value of.
//!
//! The examples in `examples/` show the loops whole.
//! `cargo run --example demux -- movie.mp4` seeks through a file
//! to what the demuxer wants, and `demux_stream` reads a response over HTTP in
//! order; both count the samples the demuxer returns. `remux` writes the
//! samples a demuxer reads into another file structure with a [`Muxer`]:
//! `cargo run --example remux -- movie.mp4 fragmented.mp4 fragmented`.
//! `fast_start` writes a movie file again with its `moov` before its media
//! data, as a [`FastStart`] lays it out:
//! `cargo run --example fast_start -- movie.mp4 fast.mp4`. `remux
//! --recoverable` writes a file that is not fragmented as movie fragments
//! until it finishes. `reserved_moov` writes a file whose `moov` goes into
//! space reserved before its media data, from the writers of the layers and
//! the reserving functions of [`framing`](isobmff::framing):
//! `cargo run --example reserved_moov -- movie.mp4 reserved.mp4 --reserve 4096`.
//! `presentation_times` lists where the edit list of each track presents
//! its samples, as a [`Timeline`](isobmff::timeline::Timeline) maps them.

#![no_std]

extern crate alloc;

mod convert;
mod demuxer;
mod error;
mod fast_start;
mod file_structure;
mod limits;
mod muxer;

pub use demuxer::{DemuxEvent, Demuxer};
pub use error::FastStartError;
pub use fast_start::{FastStart, FastStartLimits};
pub use file_structure::{FileStructure, NonFragmentedOptions};
pub use limits::{DemuxLimits, MuxLimits};
pub use muxer::{MuxError, Muxer};
