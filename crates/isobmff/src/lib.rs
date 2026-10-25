//! Sans-IO reading and writing of ISO base media files (ISO/IEC 14496-12)
//!
//! The items of [`isobmff_core`], the vocabulary a box is written in, are at
//! the root of this crate. The rest is being migrated from an earlier
//! implementation.

#![no_std]

pub use isobmff_core::*;
