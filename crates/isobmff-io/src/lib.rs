//! Demuxers and muxers driving the readers and writers of an ISO base media file over an I/O
//!
//! What this crate settles is where the bytes come from and go to; the layers
//! beneath settle everything else. A demuxer reads a file off a source a cut
//! at a time and hands each cut to the reader beneath, seeks to the bytes the
//! reader names as lacking wherever the file passed them by, and hands over
//! the samples as they come whole; a muxer takes the boxes and the samples the
//! writer beneath takes, and lays down on the sink every byte that writer
//! makes of them. One of each stands per structure of [`isobmff_structure`]:
//! a fragmented movie file (ISO/IEC 14496-12 Annex A.8), a non-fragmented one
//! (§8.2.1 and §8.7), and a media segment (§8.16).
//!
//! The I/O a driver stands over settles where it lives: every driver over
//! `std::io` is in [`blocking`], and the crate root holds the ones over
//! `futures::io`. The verbs of the two are the same names, the asynchronous
//! ones as `async fn`, and every `async fn` of this crate is cancellation
//! safe: a demuxer future dropped where the source stood still is carried on
//! by the call that follows, and a muxer verb makes its step before it awaits
//! anything, so one dropped where the sink stood still has made that step and
//! no more and the call that follows lays down what was left over — no byte
//! read or written twice, and none lost. [`Error`] is what any of them
//! reports — a file that does not read or write failed at the source or the
//! sink, or at the layers beneath.
//!
//! # `std`
//!
//! This is the one crate of the workspace that needs `std`: a source, a sink
//! and a seek are what it is for. The six layers beneath it are `no_std`.

extern crate alloc;

mod driver;
mod error;
mod fragmented_movie;
mod media_segment;
mod movie_fragment_random_access;
mod non_fragmented_movie;
mod stack;

pub mod blocking;

pub use error::{Error, ErrorKind};
pub use fragmented_movie::{FragmentedDemuxer, FragmentedMuxer};
pub use media_segment::{MediaSegmentDemuxer, MediaSegmentMuxer};
pub use non_fragmented_movie::{NonFragmentedDemuxer, NonFragmentedMuxer};
