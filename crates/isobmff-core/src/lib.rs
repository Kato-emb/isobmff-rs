//! Minimal, dependency-free layer for implementing ISO base media file format boxes
//!
//! A crate that defines boxes of its own — a vendor box, or one from a
//! specification derived from ISO/IEC 14496-12 — depends on this crate alone.
//! It holds the vocabulary such a box is written in: the identifiers that name
//! a box, the values its fields carry, and the [`Error`] its reading and
//! writing report.
//!
//! # `no_std`
//!
//! The crate is `no_std`. The `alloc` feature, on by default, adds the values
//! that own their bytes on the heap: [`Utf8String`]. Nothing else in the crate
//! reaches for a heap.

#![no_std]

#[cfg(any(feature = "alloc", test))]
extern crate alloc;

mod data_types;
mod error;
mod framing;

#[cfg(feature = "alloc")]
pub use data_types::Utf8String;
pub use data_types::{FourCc, FullBoxFlags, FullBoxHeader, I8F8, I16F16, U16F16, Uuid};
pub use error::{Category, Error, ErrorKind};
pub use framing::{BoxType, CompactType, ExtendedType};
