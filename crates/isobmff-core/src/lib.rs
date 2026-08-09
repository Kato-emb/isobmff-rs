//! Minimal, dependency-free extension layer for ISO base media file format boxes
//!
//! # `no_std`
//!
//! The crate is `no_std`. The `alloc` feature, on by default, adds the items
//! that own the bytes they carry — [`AnyBox`] and [`Utf8CString`], and the
//! [`DecodeError::Child`] variant that nests one failure inside another.
//! Nothing else in the crate reaches for a heap.

#![no_std]

#[cfg(any(feature = "alloc", test))]
extern crate alloc;

#[cfg(feature = "alloc")]
mod any_box;
mod box_decode;
mod box_definition;
mod box_encode;
mod box_framer;
mod box_header;
mod box_size;
mod box_type;
mod box_write;
mod fourcc;
mod full_box;
mod raw_box;
#[cfg(feature = "alloc")]
mod utf8_c_string;
mod uuid;

#[cfg(feature = "alloc")]
pub use any_box::AnyBox;
pub use box_decode::{BoxDecode, DecodeError};
pub use box_definition::BoxDefinition;
pub use box_encode::{BoxEncode, EncodeError};
pub use box_framer::{BoxEvent, BoxFramer, BoxFramerError};
pub use box_header::{BoxHeader, BoxHeaderError};
pub use box_size::{BoxSize, CompactSize, ExtendedSize};
pub use box_type::{BoxType, CompactType};
pub use box_write::BoxWrite;
pub use fourcc::FourCC;
pub use full_box::{FullBoxFields, FullBoxFlags};
pub use raw_box::{Boxes, RawBox, RawBoxError, boxes};
#[cfg(feature = "alloc")]
pub use utf8_c_string::Utf8CString;
pub use uuid::Uuid;
