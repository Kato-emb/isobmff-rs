//! Sans-IO reading and writing of ISO base media files (ISO/IEC 14496-12)
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
mod error;
pub mod framing;

pub use byte_range::ByteRange;
pub use error::{DemuxError, MuxError};
pub use isobmff_core::*;

/// Boxes of ISO/IEC 14496-12, each read into a value that owns its bytes
///
/// These are the items of [`isobmff_boxes`].
pub mod boxes {
    pub use isobmff_boxes::*;
}
