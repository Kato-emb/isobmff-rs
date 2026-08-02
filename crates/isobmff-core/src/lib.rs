//! Minimal, dependency-free extension layer for ISO base media file format boxes
//!
//! # `no_std`
//!
//! The crate is `no_std` and does not depend on `alloc`.

#![no_std]

#[cfg(test)]
extern crate alloc;

mod box_header;
mod box_size;
mod box_type;
mod fourcc;
mod raw_box;
mod uuid;

pub use box_header::{BoxHeader, DecodeError};
pub use box_size::{BoxSize, CompactSize, ExtendedSize};
pub use box_type::{BoxType, CompactType};
pub use fourcc::FourCC;
pub use raw_box::{Boxes, RawBox, boxes};
pub use uuid::Uuid;
