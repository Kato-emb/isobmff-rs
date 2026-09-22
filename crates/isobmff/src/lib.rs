//! Sans-IO readers and writers for an ISO base media file and the samples it carries
//!
//! A presentation is carried as samples — ISO/IEC 14496-12 §3.1.14 has a sample
//! as all the data associated with a single timestamp. [`FragmentedReader`]
//! takes a fragmented movie file as it arrives and reports the [`Sample`]s it
//! carries, [`NonFragmentedReader`] does the same for a non-fragmented one,
//! and [`MediaSegmentReader`] for a media segment delivered apart from the
//! movie it continues; [`FragmentedWriter`], [`NonFragmentedWriter`] and
//! [`MediaSegmentWriter`] go the other way, laying samples down as a file or
//! a segment of each kind. None reaches for a source or a sink of its own:
//! when to read or write, and from or to where, stay with the caller. Where
//! the caller has `std::io` to hand, the `std` feature drives each of them:
//! [`FragmentedDemuxer`], [`NonFragmentedDemuxer`] and [`MediaSegmentDemuxer`]
//! read one off a `Read + Seek`, [`FragmentedMuxer`], [`NonFragmentedMuxer`]
//! and [`MediaSegmentMuxer`] write one to a `Write`.
//!
//! # Everything in one place
//!
//! The crates this one is built on are re-exported whole, so a caller reaching
//! for any layer names `isobmff` alone: [`isobmff_core`] for the framing and
//! the field codecs, [`isobmff_boxes`] for the catalog of boxes,
//! [`isobmff_sample`] for the sample layers, [`isobmff_structure`] for the
//! structures a file is read and written through and the stacks that drive
//! them. Their names are re-exported as they stand, so documentation written
//! against any of them reads against this one. The seven layers a file is
//! read through, and which crate holds each, are [`isobmff_structure`]'s to
//! describe.
//!
//! The names that could not stand are [`Error`] and [`ErrorKind`], which
//! [`isobmff_core`] holds: the failures of the framing are re-exported as
//! [`SequenceError`] and [`SequenceErrorKind`], and the failures of the sample
//! layers are [`SampleError`] and of the structures [`StructureError`], both
//! named apart at the source rather than shadowed.
//!
//! The sample entries other specifications define over ISO/IEC 14496-12 sit in
//! a module per specification — [`avc`] for ISO/IEC 14496-15, [`mp4`] for
//! ISO/IEC 14496-14 — each behind the Cargo feature of the same name, on by
//! default. A caller that wants the base specification alone turns the default
//! features off.
//!
//! # `no_std`
//!
//! The crate is `no_std` but needs `alloc`, for the reasons
//! [`isobmff_structure`] states of the layers beneath. The `std` feature, off
//! by default, adds the demuxers and the muxers alone: the six layers beneath
//! them are the same with it or without.

#![no_std]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "std")]
mod driver;
#[cfg(feature = "std")]
mod driver_error;
#[cfg(feature = "std")]
mod fragmented_movie;
#[cfg(feature = "std")]
mod media_segment;
#[cfg(feature = "std")]
mod non_fragmented_movie;

#[cfg(feature = "std")]
pub use driver_error::{DriverError, DriverErrorKind};
#[cfg(feature = "std")]
pub use fragmented_movie::{FragmentedDemuxer, FragmentedMuxer};
#[cfg(feature = "std")]
pub use media_segment::{MediaSegmentDemuxer, MediaSegmentMuxer};
#[cfg(feature = "std")]
pub use non_fragmented_movie::{NonFragmentedDemuxer, NonFragmentedMuxer};

#[cfg(feature = "std")]
pub(crate) use driver::{Demuxer, Muxer, PollOutput, ReadSamples};

pub use isobmff_boxes::*;
pub use isobmff_core::*;
pub use isobmff_sample::*;
pub use isobmff_sequence::{
    BoxEvent, BoxReader, BoxWriter, Error as SequenceError, ErrorKind as SequenceErrorKind,
    EventBytes,
};
pub use isobmff_structure::*;

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
