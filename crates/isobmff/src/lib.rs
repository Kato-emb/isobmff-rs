//! Sans-IO reading and writing of ISO base media files (ISO/IEC 14496-12)
//!
//! [`file_level`] reads the boxes at `file level` from bytes that arrive in
//! pieces. [`structure_data`] reads what the tables of a sample table declare
//! of its samples: their timing, and where their bytes lie, as a data offset
//! from a [`DataOrigin`] in the container a data reference entry specifies;
//! its [`MovieReader`](structure_data::MovieReader) reads them for every
//! track of a `moov`.
//! [`SampleProperties`] and [`SampleLocation`] hold what it declares of one
//! sample. The boxes themselves are read and written with [`boxes`], and the
//! header of a box, the values its fields carry and the traits a box
//! implements, such as [`BoxDecode`] and [`BoxEncode`], are at the root of
//! the crate. The [`Error`] at the root is that of reading and writing a box.

#![no_std]

extern crate alloc;

pub mod file_level;
mod sample;
pub mod structure_data;

/// The boxes of ISO/IEC 14496-12, each read into a value and written back, from `isobmff-boxes`
pub use isobmff_boxes as boxes;
pub use isobmff_core::*;
pub use sample::{DataOrigin, SampleLocation, SampleProperties};

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
