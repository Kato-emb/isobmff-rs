//! Sans-IO reading and writing of ISO base media files (ISO/IEC 14496-12)
//!
//! The items of [`isobmff_core`], the vocabulary a box is written in, are at
//! the root of this crate. The boxes ISO/IEC 14496-12 defines, written in that
//! vocabulary, are in [`boxes`].
//!
//! [`structure_data`] resolves the `moov` and the `moof`s of a file to the
//! [`SampleLocation`] of each sample they declare, and [`media_data`]
//! gathers the bytes of those samples out of the bytes of the file.
//!
//! The examples in `crates/isobmff/examples/` read and write files through
//! [`framing`]: `cargo run -p isobmff --example dump_boxes -- movie.mp4` lists
//! the boxes of a file, and `split_fragments` splits a fragmented movie file
//! into segments.

#![no_std]

extern crate alloc;

mod byte_range;
mod error;
pub mod framing;
pub mod media_data;
mod sample;
pub mod structure_data;

pub use byte_range::ByteRange;
pub use error::{DemuxError, MuxError};
pub use isobmff_core::*;
pub use sample::{Sample, SampleLocation, SampleProperties};

/// Boxes of ISO/IEC 14496-12, each read into a value that owns its bytes
///
/// These are the items of [`isobmff_boxes`].
pub mod boxes {
    pub use isobmff_boxes::*;
}
