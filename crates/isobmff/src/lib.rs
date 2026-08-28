//! Sans-IO reader and writer for the samples an ISO base media file carries in movie fragments
//!
//! A presentation is carried as samples — ISO/IEC 14496-12 §3.1.14 has a sample
//! as all the data associated with a single timestamp. [`SampleReader`] takes a
//! fragmented presentation as it arrives, the movie fragments and the media data
//! beside them, and reports the [`Sample`]s they carry: where each one lies is
//! resolved from the offsets a fragment states, and what is true of it from the
//! defaults the fragment and the track set. [`SampleWriter`] goes the other way,
//! laying samples out as the `moof` and `mdat` of one fragment after another.
//! Neither reaches for a source or a sink of its own: when to read or write, and
//! from or to where, stay with the caller.
//!
//! # Taking a file apart, and putting one together
//!
//! This crate reads and writes samples, not files. Framing a file into boxes is
//! the work of `isobmff-sequence`, a layer beside this one rather than beneath
//! it: neither knows the other, and the caller wires them together. A
//! `BoxReader` reports a `moof` read into a value and the payload of an `mdat` as
//! it lies, which is what the two `handle_*` calls here take, so the wiring is a
//! match on two of its events; a `BoxWriter` takes the pair a fragment is
//! written as. A caller holding the presentation in memory has no need of it at
//! all: [`boxes`] frames it, and the samples read from there just the same.
//!
//! Both layers report where a box lay in what they were handed, counting from
//! the first byte that arrived. This one is handed those extents resolved
//! against the origin the presentation was read from, which is the caller's to
//! add.
//!
//! # Everything in one place
//!
//! The crates this one is built on are re-exported whole, so a caller reaching
//! for the box layer or the traits beneath it names `isobmff` alone:
//! [`isobmff_core`] for the framing and the field codecs, [`isobmff_boxes`] for
//! the catalog of boxes. Their names are re-exported as they stand, so
//! documentation written against either crate reads against this one.
//!
//! The one name that could not stand is [`Error`], which [`isobmff_core`] holds:
//! the failures of this layer are [`SampleError`] and [`SampleErrorKind`], named
//! apart at the source rather than shadowed here.
//!
//! The sample entries other specifications define over ISO/IEC 14496-12 are
//! re-exported as a module per specification rather than flat — [`avc`] for
//! ISO/IEC 14496-15 and [`mp4`] for ISO/IEC 14496-14 — so each keeps the names
//! its own document gives, `mp4::Error` among them, and a specification added
//! later adds a module rather than a crowd of names. Each module sits behind
//! the Cargo feature of the same name, on by default; a caller that wants the
//! base specification alone turns the default features off.
//!
//! # `no_std`
//!
//! The crate is `no_std` but needs `alloc`: a sample owns the bytes it carries,
//! the claims of a fragment are held until the data that meets them arrives, and
//! the samples of a fragment being written are held until it is closed.

#![no_std]

extern crate alloc;

mod error;
mod reader;
mod sample;
mod writer;

pub use error::{SampleError, SampleErrorKind};
pub use reader::SampleReader;
pub use sample::Sample;
pub use writer::SampleWriter;

pub use isobmff_boxes::*;
pub use isobmff_core::*;

/// Sample entries of ISO/IEC 14496-15, the carriage of AVC video, behind the
/// `avc` feature
#[cfg(feature = "avc")]
pub use isobmff_avc as avc;
/// Sample entries and descriptors of ISO/IEC 14496-14, the MP4 file format,
/// behind the `mp4` feature
#[cfg(feature = "mp4")]
pub use isobmff_mp4 as mp4;
