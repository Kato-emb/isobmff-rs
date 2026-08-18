//! Sans-IO reader for the sequence of boxes an ISO base media file is formed as
//!
//! A file is structured as a sequence of objects, called boxes — ISO/IEC
//! 14496-12 §4.2. [`BoxReader`] takes that sequence as it arrives in chunks of any length and
//! reports it as [`BoxEvent`]s, each owning the bytes it carries. It reaches for
//! no source and no destination: chunks are handed to it and events are taken
//! from it, leaving when and from where to read to its caller.
//!
//! # `no_std`
//!
//! The crate is `no_std` but needs `alloc`: an event owns the bytes it carries,
//! and the events a chunk completed are held until the caller takes them.

#![no_std]

extern crate alloc;

mod reader;

pub use reader::{BoxEvent, BoxReader, BoxReaderError};
