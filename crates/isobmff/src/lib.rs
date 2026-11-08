//! Sans-IO reading and writing of ISO base media files (ISO/IEC 14496-12)
//!
//! Under construction. [`framing`] reads the boxes laid end to end in a file
//! from bytes that arrive in pieces; the boxes themselves are read and written
//! with `isobmff-boxes`.

#![no_std]

extern crate alloc;

pub mod framing;
mod input_buffer;

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
