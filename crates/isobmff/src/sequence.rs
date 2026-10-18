//! A file framed as the sequence of boxes it is
//!
//! A file is structured as a sequence of objects, called boxes — ISO/IEC
//! 14496-12 §4.2. [`BoxReader`] takes that sequence as it arrives, cut
//! anywhere, and reports it as [`BoxEvent`]s, each owning the bytes it carries.
//! [`BoxWriter`] is the mirror of it: handed those events, it lays the sequence
//! back down as bytes. Every box is carried as it lies: this module frames a
//! file and reads no box into a value, so which boxes matter and what their
//! payloads mean stay with the caller. Neither reaches for a source or a
//! destination: input is handed over and output is taken, leaving when and from
//! where to read, and where to write, to the caller.
//!
//! An event says what the file holds and not where it holds it. Where it lies is
//! the extent each of the two hands over with it —
//! [`BoxReader::poll_event`] and [`BoxWriter::handle_event`] — a contiguous
//! subset of the bytes of a resource, ISO/IEC 14496-12 §8.11.3. It counts from
//! the first byte handed over, so a file handed over from its first byte has
//! every extent a file offset, the coordinate the boxes declare their own in
//! (§8.7.5, §8.8.7).
//!
//! # What this layer does not know
//!
//! Neither the box values of [`boxes`](crate::boxes) nor the layers that read
//! them ([`sample`](crate::sample), [`structure`](crate::structure)) are named
//! here: a box is carried as it lies, and what it means is left to the
//! layers above.
//!
//! # `alloc`
//!
//! The layer here needs `alloc`: an event owns the box or the bytes it
//! carries, and what a call completed is held until the caller takes it.

#[cfg(test)]
pub(crate) mod driving;
mod error;
mod event;
mod reader;
mod writer;

pub use error::Error;
pub use event::{BoxEvent, OutputBytes};
pub use reader::BoxReader;
pub use writer::BoxWriter;
