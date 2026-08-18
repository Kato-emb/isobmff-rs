//! Sans-IO reader for the sequence of boxes an ISO base media file is formed as
//!
//! A file is structured as a sequence of objects, called boxes — ISO/IEC
//! 14496-12 §4.2. [`BoxReader`] takes that sequence as it arrives, cut anywhere,
//! and reports it as [`BoxEvent`]s, each owning the bytes it carries. The boxes
//! a file is framed by are read into values, and every other box is passed on as
//! it lies. It reaches for no source and no destination: input is handed to it
//! and events are taken from it, leaving when and from where to read to its
//! caller.
//!
//! # `no_std`
//!
//! The crate is `no_std` but needs `alloc`: an event owns the box or the bytes
//! it carries, and the events the input completed are held until the caller takes
//! them.

#![no_std]

extern crate alloc;

mod event;
mod reader;

pub use event::BoxEvent;
pub use reader::{BoxReader, BoxReaderError};
