//! Sans-IO sample layer of an ISO base media file: where its samples lie, and the samples themselves
//!
//! A presentation is carried as samples — ISO/IEC 14496-12 §3.1.14 has a sample
//! as all the data associated with a single timestamp. A file declares where
//! each sample lies in one of two forms, the sample table of a movie (§8.7) or
//! the track runs of a movie fragment (§8.8), and this crate speaks one
//! vocabulary for both.
//!
//! # The two layers this crate holds
//!
//! * **Sample resolution.** [`SampleExtent`] is what a declaration of either
//!   form resolves to: the properties of a sample and the extent of the file
//!   its bytes lie in. [`movie_fragment::sample_extents`] resolves a `moof`
//!   against its movie — the `trex` and the `stsd` of each track (§8.8); it
//!   holds no state across calls, and what §8.8 carries from one fragment to
//!   the next — where each track's decode time stands when a fragment states
//!   none (§8.8.12) — is a [`TrackDecodeTimes`] the caller owns and hands in.
//! * **Sample gathering.** [`Sample`] is a [`SampleExtent`] joined with the
//!   bytes it names.
//!
//! Neither layer frames a box or reaches for the file: the boxes come in as
//! values and the bytes as slices, so where either is read from stays with the
//! caller.
//!
//! What either layer fails on is a [`SampleError`].
//!
//! # `no_std`
//!
//! The crate is `no_std` but needs `alloc`: a sample owns the bytes it carries,
//! and the extents declared are held until the bytes that meet them arrive.

#![no_std]

extern crate alloc;

mod error;
pub mod movie_fragment;
mod sample;
mod track_decode_times;

pub use error::{SampleError, SampleErrorKind};
pub use sample::{Sample, SampleExtent};
pub use track_decode_times::TrackDecodeTimes;
