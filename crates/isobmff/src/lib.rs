//! Sans-IO reading and writing of ISO base media files (ISO/IEC 14496-12), in layers that follow the structure of a file
//!
//! Each layer does no I/O and is usable alone in either direction: a reader
//! names the bytes it wants next and takes them with the file offset they
//! start at, and a writer returns bytes with the file offset they go to.
//!
//! | Layer | Reads | Writes | ISO/IEC 14496-12 |
//! |---|---|---|---|
//! | [`framing`] | the boxes laid end to end, out of bytes that arrive in pieces | the boxes laid end to end as bytes | §4.2.2 |
//! | [`file_level`] | the kind of each box at file level, and the value of each that has one | | A.3 |
//! | [`structure_data`] | the `moov` and `moof`s, resolved to the [`SampleLocation`] of each sample | the sample tables of the `moov`, from the [`SampleLocation`] of each sample, or a `moof` and the [`SampleLocation`] it gives each | A.3, §8.5, §8.8 |
//! | [`media_data`] | the bytes of each sample, gathered out of the bytes of the file | the `mdat` of samples, and the [`SampleLocation`] of each | §8.1.1 |
//! | [`timeline`] | where the edit list of a track presents its samples | | §8.6.6 |
//!
//! A caller that reads or writes files whole uses the crate `isobmff-mux`,
//! made of these layers: it reads a file into its samples, writes samples
//! into a movie file or a segment, and lays a movie file out again with its
//! `moov` first. A reader or a writer of its own is made of the layers as
//! that crate is. This one reads the samples of a movie file whose media
//! data is in the file itself:
//!
//! ```
//! use isobmff::file_level::{Kind, TopLevelBox};
//! use isobmff::framing::{BoxEvent, BoxReader};
//! use isobmff::structure_data::{StructureDataLimits, StructureDataReader};
//!
//! let file: &[u8] = include_bytes!("../tests/data/movie.mp4");
//! # let limits = StructureDataLimits {
//! #     sample_count: 1024,
//! #     segment_index_count: 0,
//! #     subsegment_count: 0,
//! #     leniency_count: 0,
//! #     identified_media_data_count: 0,
//! #     unresolved_sample_count: 0,
//! #     compressed_box_count: 0,
//! # };
//! let (mut boxes, mut structure_data) = (BoxReader::new(), StructureDataReader::new(limits));
//! let mut locations = Vec::new();
//! while let Some(wanted) = boxes.wanted_input() {
//!     let input = file.get(usize::try_from(wanted.offset())?..).unwrap_or_default();
//!     if input.is_empty() {
//!         break;
//!     }
//!     while let Some(event) = boxes.handle_input(wanted.offset(), input)? {
//!         match event {
//!             // Gather the structure-data, and skip the media data
//!             BoxEvent::Header(header) => match Kind::of(header.box_type()) {
//!                 Kind::StructureData => boxes.gather(1024 * 1024)?,
//!                 Kind::MediaData | Kind::IdentifiedMediaData | Kind::Other => boxes.skip()?,
//!             },
//!             // Resolve the samples of the `moov` to where their media data lies
//!             BoxEvent::End => {
//!                 if let Some(TopLevelBox::Movie(moov)) = boxes.gathered_box().map(TopLevelBox::read).transpose()?.flatten() {
//!                     locations.extend(structure_data.handle_movie(moov)?);
//!                 }
//!             }
//!             BoxEvent::Payload(_) => {}
//!         }
//!     }
//! }
//!
//! // Read the media data of each sample where it lies
//! let samples: Vec<&[u8]> = locations.iter().map(|location| {
//!     let range = location.byte_range();
//!     file.get(usize::try_from(range.start)?..usize::try_from(range.end)?).ok_or("short".into())
//! }).collect::<Result<_, Box<dyn core::error::Error>>>()?;
//! assert_eq!(samples, [b"AB", b"CD"]);
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! Where the tables of a sample table state different numbers of samples,
//! the structure-data reader reads the file as conforming readers do rather
//! than refuse it, and keeps each such [`Leniency`] for the caller to ask
//! after with
//! [`StructureDataReader::leniencies`](structure_data::StructureDataReader::leniencies).
//!
//! The feature `compressed-boxes` reads the compressed boxes (ISO/IEC
//! 14496-12 §8.16), whose payload is the deflated payload of the box they
//! replace: `framing::decompressed` reads one back into that box, and
//! [`file_level`] reads the brands of a file from its `otyp`.
//!
//! The items of [`isobmff_core`], the vocabulary a box is written in, are at
//! the root of this crate. The boxes ISO/IEC 14496-12 defines, written in that
//! vocabulary, are in [`boxes`].
//!
//! The examples in `examples/` show the loops whole. `dump_boxes` and
//! `split_fragments` read and write files through [`framing`]: one lists the
//! boxes of a file, those a compressed box replaces among them, the other
//! splits a fragmented movie file into segments:
//! `cargo run --example dump_boxes -- movie.mp4`.

#![no_std]

extern crate alloc;

mod byte_range;
mod convert;
mod error;
pub mod file_level;
pub mod framing;
mod leniency;
pub mod media_data;
mod output;
mod sample;
pub mod structure_data;
pub mod timeline;

pub use byte_range::ByteRange;
pub use error::{ReadError, TimelineError, WriteError};
#[doc(inline)]
pub use file_level::supports_default_base_is_moof;
pub use isobmff_core::*;
pub use leniency::{Leniency, SampleTableDisagreement};
pub use output::Output;
pub use sample::{DataReferenceKey, Sample, SampleLocation, SampleProperties};

/// Boxes of ISO/IEC 14496-12, each read into a value that owns its bytes
///
/// These are the items of [`isobmff_boxes`].
pub mod boxes {
    pub use isobmff_boxes::*;
}
