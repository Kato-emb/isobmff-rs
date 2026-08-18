//! Sans-IO reader and writer for the sequence of boxes an ISO base media file is formed as
//!
//! A file is structured as a sequence of objects, called boxes — ISO/IEC
//! 14496-12 §4.2. [`BoxReader`] takes that sequence as it arrives, cut anywhere,
//! and reports it as [`BoxEvent`]s, each owning the bytes it carries and wrapped
//! in the [`BoxEventAt`] naming where in the file it begins. [`BoxWriter`] is the
//! mirror of it: handed those events, it lays the sequence back down as bytes.
//! The boxes a file is framed by are read into values, and every other box is
//! carried as it lies. Neither reaches for a source or a destination: input is
//! handed over and output is taken, leaving when and from where to read, and
//! where to write, to the caller.
//!
//! # `no_std`
//!
//! The crate is `no_std` but needs `alloc`: an event owns the box or the bytes
//! it carries, and what a call completed is held until the caller takes it.

#![no_std]

extern crate alloc;

mod event;
mod reader;
mod writer;

pub use event::{BoxEvent, BoxEventAt};
pub use reader::{BoxReader, BoxReaderError};
pub use writer::{BoxWriter, BoxWriterError};
