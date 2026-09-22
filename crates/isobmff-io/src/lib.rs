//! Demuxers and muxers driving the readers and writers of an ISO base media file over an I/O
//!
//! Where the bytes come from and go to is what this crate settles, and the
//! layers beneath it settle everything else: the file is handed over from its
//! first byte, output is taken, and what a reader says it still lacks is
//! fetched or not. A demuxer reads a file off a source a cut at a time and
//! hands each cut to the reader beneath, seeks to the bytes the reader names
//! as lacking wherever the file passed them by, and yields the samples as
//! they come whole; a muxer takes the boxes and the samples a writer takes,
//! and lays down on the sink every byte the writer makes of them. One of each
//! stands per structure of [`isobmff_structure`]: a fragmented movie file
//! (ISO/IEC 14496-12 Annex A.8), a non-fragmented one (§8.2.1 and §8.7), and a
//! media segment (§8.16).
//!
//! Which I/O carries a driver settles where it stands: every driver over
//! `std::io` is in [`blocking`], and the crate root is kept for the
//! asynchronous ones. [`Error`] is common to both — a file that does not read
//! or write failed at the source or the sink, or at the layers beneath.

extern crate alloc;

mod error;

pub mod blocking;

pub use error::{Error, ErrorKind};
