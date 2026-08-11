//! Minimal, dependency-free extension layer for ISO base media file format boxes
//!
//! # `no_std`
//!
//! The crate is `no_std`. The `alloc` feature, on by default, adds the items
//! that need a heap: [`AnyBox`] and [`NullTerminatedString`], which own the
//! bytes they carry, [`ChildBoxes`] and [`OtherBoxes`], which gather the
//! children of a container, and the [`DecodeError::Child`] variant that nests
//! one failure inside another. Nothing else in the crate reaches for a heap.

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
#[cfg(feature = "alloc")]
mod container;
mod data_types;
mod field;
mod fourcc;
mod full_box;
mod language_code;
#[cfg(feature = "alloc")]
mod null_terminated_string;
mod raw_box;
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
#[cfg(feature = "alloc")]
pub use container::{ChildBoxes, OtherBoxes};
pub use data_types::{I8F8, I16F16, Matrix, QuickTimeDateTime, U16F16};
pub use field::{FieldReadError, FieldReader, FieldWriteError, FieldWriter};
pub use fourcc::FourCC;
pub use full_box::{FullBoxFields, FullBoxFlags};
pub use language_code::LanguageCode;
#[cfg(feature = "alloc")]
pub use null_terminated_string::NullTerminatedString;
pub use raw_box::{Boxes, RawBox, RawBoxError, boxes};
pub use uuid::Uuid;
