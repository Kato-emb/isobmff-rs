//! Minimal, dependency-free extension layer for ISO base media file format boxes
//!
//! # `no_std`
//!
//! The crate is `no_std`. The `alloc` feature, on by default, adds the items
//! that need a heap: [`AnyBox`] and [`NullTerminatedString`], which own the
//! bytes they carry, and [`ChildBoxes`] and [`OtherBoxes`], which gather the
//! children of a container. Nothing else in the crate reaches for a heap.

#![no_std]

#[cfg(any(feature = "alloc", test))]
extern crate alloc;

#[cfg(feature = "alloc")]
mod any_box;
mod codec;
#[cfg(feature = "alloc")]
mod container;
mod data_types;
mod error;
mod framing;

#[cfg(feature = "alloc")]
pub use any_box::AnyBox;
pub use codec::{
    BoxDecode, BoxDefinition, BoxEncode, BoxRead, BoxWrite, FieldReader, FieldWidth, FieldWriter,
};
#[cfg(feature = "alloc")]
pub use container::{ChildBoxes, OtherBoxes};
#[cfg(feature = "alloc")]
pub use data_types::NullTerminatedString;
pub use data_types::{
    FourCC, FullBoxFields, FullBoxFlags, I8F8, I16F16, LanguageCode, Matrix, Mp4EpochSeconds,
    U16F16, Uuid,
};
pub use error::{Category, Error, ErrorKind};
pub use framing::{
    BoxHeader, BoxSize, BoxType, Boxes, CompactSize, CompactType, ExtendedSize, RawBox, boxes,
};
