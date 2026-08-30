//! Sans-IO reader and writer for a fragmented ISO base media file and the samples it carries
//!
//! A presentation is carried as samples — ISO/IEC 14496-12 §3.1.14 has a sample
//! as all the data associated with a single timestamp. [`FragmentedReader`]
//! takes a fragmented movie file as it arrives and reports the [`Sample`]s it
//! carries; [`FragmentedWriter`] goes the other way, laying samples down as such
//! a file. Neither reaches for a source or a sink of its own: when to read or
//! write, and from or to where, stay with the caller.
//!
//! # The layers a file is read through
//!
//! Four layers stand between a file and the samples it carries. Three of them
//! are this crate's, each holding one clause of the specification; the fourth is
//! the reading and writing itself, which holds none and stays with the caller.
//!
//! * **The boxes.** A file is a sequence of objects, called boxes (§4.2), and
//!   framing that sequence is the work of [`BoxReader`] and [`BoxWriter`], which
//!   `isobmff-sequence` holds. They read no box into a value: which boxes matter
//!   is not theirs to say.
//! * **The layout.** [`FragmentedReader`] and [`FragmentedWriter`] hold the
//!   layout of a fragmented movie file (§8.8, Annex A.8) — the brands, the
//!   movie, then one movie fragment after another with the media data beside it.
//!   Knowing the layout is what settles which boxes are read into values, how
//!   much payload may be gathered for one, and the order they come in.
//! * **The samples.** [`SampleReader`] resolves where every sample of a movie
//!   fragment lies and what is true of it (§8.8.7, §8.8.8, §8.8.12), and
//!   [`SampleWriter`] lays samples out as the `moof` and `mdat` of one fragment.
//!   Both are scoped to a fragment and carry no notion of a file, so the same
//!   pair serves a media segment as it serves a file.
//! * **The I/O.** Where the bytes come from and go to is the caller's: input is
//!   handed over and output is taken, so a `File`, a socket, or a buffer already
//!   in memory drives the three layers above the same way.
//!
//! A caller that holds a whole presentation in memory needs none of the layers:
//! [`boxes`] frames it, and the samples read from there just the same.
//!
//! Where a box lay is reported by the box layer as an extent counting from the
//! first byte handed over, and the sample layer resolves the offsets a fragment
//! declares against it. The layout layer passes them between the two itself, so
//! a caller of it hands over bytes and takes samples and never sees one.
//!
//! # Everything in one place
//!
//! The crates this one is built on are re-exported whole, so a caller reaching
//! for the box layer or the traits beneath it names `isobmff` alone:
//! [`isobmff_core`] for the framing and the field codecs, [`isobmff_boxes`] for
//! the catalog of boxes. Their names are re-exported as they stand, so
//! documentation written against either crate reads against this one.
//!
//! The names that could not stand are [`Error`] and [`ErrorKind`], which
//! [`isobmff_core`] holds: the failures of the framing are re-exported as
//! [`SequenceError`] and [`SequenceErrorKind`], and the failures of the two
//! layers named here are [`FileError`] and [`SampleError`], named apart at the
//! source rather than shadowed.
//!
//! The sample entries other specifications define over ISO/IEC 14496-12 sit in
//! a module per specification — [`avc`] for ISO/IEC 14496-15, [`mp4`] for
//! ISO/IEC 14496-14 — each behind the Cargo feature of the same name, on by
//! default. A caller that wants the base specification alone turns the default
//! features off.
//!
//! # `no_std`
//!
//! The crate is `no_std` but needs `alloc`: a sample owns the bytes it carries,
//! the claims of a fragment are held until the data that meets them arrives, and
//! the samples of a fragment being written are held until it is closed.

#![no_std]

extern crate alloc;

mod error;
mod file_error;
mod fragmented_reader;
mod fragmented_writer;
mod reader;
mod sample;
mod writer;

pub use error::{SampleError, SampleErrorKind};
pub use file_error::{FileError, FileErrorKind};
pub use fragmented_reader::FragmentedReader;
pub use fragmented_writer::FragmentedWriter;
pub use reader::SampleReader;
pub use sample::Sample;
pub use writer::SampleWriter;

pub use isobmff_boxes::*;
pub use isobmff_core::*;
pub use isobmff_sequence::{
    BoxEvent, BoxReader, BoxWriter, Error as SequenceError, ErrorKind as SequenceErrorKind,
    EventBytes,
};

// Why not `pub use isobmff_avc as avc`: rustdoc renders that as one line under
// Re-exports, with no module page and no `isobmff::avc::…` items to search, and
// a flat glob would let `isobmff_mp4::Error` collide with `Error` above.
/// Sample entries of ISO/IEC 14496-15, the carriage of AVC video — the
/// `isobmff-avc` crate whole, behind the `avc` feature
#[cfg(feature = "avc")]
pub mod avc {
    pub use isobmff_avc::*;
}

/// Sample entries and descriptors of ISO/IEC 14496-14, the MP4 file format —
/// the `isobmff-mp4` crate whole, behind the `mp4` feature
#[cfg(feature = "mp4")]
pub mod mp4 {
    pub use isobmff_mp4::*;
}
