//! Sans-IO demuxers and muxers for an ISO base media file and the samples it carries
//!
//! A presentation is carried as samples — ISO/IEC 14496-12 §3.1.14 has a sample
//! as all the data associated with a single timestamp.
//! [`MovieDemuxer`] takes a movie file as it arrives, fragmented or not, and
//! reports the [`Sample`]s it carries, and [`MediaSegmentDemuxer`] does the
//! same for a media segment delivered apart from the movie it continues;
//! [`FragmentedMuxer`], [`NonFragmentedMuxer`] and [`MediaSegmentMuxer`] go
//! the other way, laying samples down as a file or a segment of each kind.
//! None reaches for a source or a sink of its own, and none does I/O: when to
//! read or write, and from or to where, stay with the caller.
//!
//! # The root and the modules
//!
//! The root holds the demuxers, the muxers and what they take and yield —
//! [`Sample`], [`SampleExtent`], [`WantedInput`], [`OutputBytes`],
//! [`DemuxLimits`], [`DemuxError`] and [`MuxError`] among them — beside the
//! items of the `isobmff-core` crate: [`BoxType`], [`FourCc`], [`BoxDecode`]
//! and the rest of what all the layers are defined over. A module the table
//! places in another crate is that crate re-exported whole:
//! [`boxes::MovieBox`] and `isobmff_boxes::MovieBox` are the same type, so
//! documentation written against any of those crates reads against this one.
//!
//! | path | crate | holds |
//! |---|---|---|
//! | the root | this crate and `isobmff-core` | the demuxers and muxers, what they take and yield, and what all the layers are defined over |
//! | [`sequence`] | this crate | a file framed as the sequence of boxes it is |
//! | [`boxes`] | `isobmff-boxes` | the catalog of boxes a box is read into |
//! | [`sample`] | this crate | where the samples of a presentation lie, and the samples themselves |
//! | [`avc`] | `isobmff-avc` | the sample entries of ISO/IEC 14496-15 |
//! | [`mp4`] | `isobmff-mp4` | the sample entries and descriptors of ISO/IEC 14496-14 |
//!
//! The I/O is the caller's code, and no module holds it. What each module
//! holds is its own summary below. `avc` and `mp4` are on by default,
//! so a caller that wants the base specification alone turns the default
//! features off.
//!
//! The root and each module with failures of its own name them `Error`; the
//! demuxers and muxers name theirs by what fails, [`DemuxError`] and
//! [`MuxError`]. Each of these carries the failures of the layers beneath through whole rather than
//! translating them, so [`DemuxError`] reaches [`sample::Error`] and
//! [`sequence::Error`] reaches [`Error`].
//!
//! # `no_std`
//!
//! The crate is `no_std` but needs `alloc`, for the reasons
//! [`sequence`](sequence#alloc) and [`sample`](sample#alloc) state of their
//! layers, and because the demuxers and muxers hold what they read and write
//! until it is whole: a box read into a value, the extents a movie or a
//! fragment declares until the data that meets them arrives, the samples of a
//! fragment or a chunk until it is laid down, and the movie a non-fragmented
//! file is written against until the file is over.
//!
//! # Examples
//!
//! The [`examples`](https://github.com/Kato-emb/isobmff-rs/tree/main/examples)
//! directory of the repository holds one program per use, each written
//! against this crate and the I/O it reads and writes through, and run from a
//! checkout as `cargo run -p isobmff-examples --example <name> -- <arguments>`.

#![no_std]

extern crate alloc;

pub use demux::{
    DemuxError, DemuxLimits, MediaSegmentDemuxer, MovieDemuxer, SampleExtents, SampleGathering,
    WantedInput,
};
pub use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxFormat, BoxHeader, BoxSize, BoxType,
    BoxVariants, Boxes, Category, ChildBoxes, CompactSize, CompactType, CompressorName, Error,
    ErrorKind, ExtendedSize, FieldReader, FieldWidth, FieldWriter, FourCc, FullBoxFields,
    FullBoxFlags, I8F8, I16F16, InContainer, LanguageCode, Matrix, Mp4EpochSeconds,
    NullTerminatedString, OtherBoxes, RawBox, U16F16, Uuid,
};
pub use mux::{FragmentedMuxer, MediaSegmentMuxer, MuxError, NonFragmentedMuxer};
pub use sample::{Sample, SampleExtent, SampleProperties};
pub use sequence::OutputBytes;

pub mod sequence;

/// The catalog of ISO/IEC 14496-12 boxes — the `isobmff-boxes` crate whole
#[doc(inline)]
pub use isobmff_boxes as boxes;

pub mod sample;

mod demux;
mod mux;
mod structure;

/// Sample entries of ISO/IEC 14496-15, the carriage of AVC video — the
/// `isobmff-avc` crate whole, behind the `avc` feature
#[cfg(feature = "avc")]
#[doc(inline)]
pub use isobmff_avc as avc;

/// Sample entries and descriptors of ISO/IEC 14496-14, the MP4 file format —
/// the `isobmff-mp4` crate whole, behind the `mp4` feature
#[cfg(feature = "mp4")]
#[doc(inline)]
pub use isobmff_mp4 as mp4;
