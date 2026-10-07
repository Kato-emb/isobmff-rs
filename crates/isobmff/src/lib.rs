//! Sans-IO demux and mux FSMs for an ISO base media file and the samples it carries
//!
//! A presentation is carried as samples — ISO/IEC 14496-12 §3.1.14 has a sample
//! as all the data associated with a single timestamp.
//! [`structure::MovieDemuxFsm`] takes a movie file as it arrives, fragmented or
//! not, and reports the [`sample::Sample`]s it carries, and
//! [`structure::MediaSegmentDemuxFsm`] does the same for a media segment
//! delivered apart from the movie it continues; [`structure::FragmentedMuxFsm`],
//! [`structure::NonFragmentedMuxFsm`] and [`structure::MediaSegmentMuxFsm`] go
//! the other way, laying samples down as a file or a segment of each kind.
//! None reaches for a source or a sink of its own: when to read or write, and
//! from or to where, stay with the caller.
//!
//! # One module per crate
//!
//! Every module here is one crate of the workspace re-exported whole, and this
//! root holds nothing else: [`structure::MovieDemuxFsm`] and
//! `isobmff_structure::MovieDemuxFsm` are the same type, so documentation
//! written against any of those crates reads against this one.
//!
//! | module | crate | of the seven layers |
//! |---|---|---|
//! | [`core`] | `isobmff-core` | what all of them are defined over |
//! | [`sequence`] | `isobmff-sequence` | layer 1 |
//! | [`boxes`] | `isobmff-boxes` | the catalog layer 2 reads a box into |
//! | [`sample`] | `isobmff-sample` | layers 3 and 4 |
//! | [`structure`] | `isobmff-structure` | layers 2, 5 and 6 |
//! | [`avc`] | `isobmff-avc` | none: the sample entries of ISO/IEC 14496-15 |
//! | [`mp4`] | `isobmff-mp4` | none: the sample entries and descriptors of ISO/IEC 14496-14 |
//!
//! Layer 7, the I/O, is the caller's code, and no module holds it: the loop
//! between a machine and its I/O is
//! [the one `isobmff_structure` states](isobmff_structure#the-callers-loop).
//! What each module holds is its own summary below; what the seven layers
//! are, and what passes between them, [`isobmff_structure`] describes.
//! `avc` and `mp4` are on by default, so a caller that wants the base
//! specification alone turns the default features off.
//!
//! Each crate names the failures of its own layers `Error`, and carries the
//! failures of the layers beneath through whole rather than translating them,
//! so [`structure::Error`] reaches [`sample::Error`] and [`sequence::Error`]
//! reaches [`core::Error`].
//!
//! # `no_std`
//!
//! The crate is `no_std` but needs `alloc`, for the reasons
//! [`isobmff_structure`] states of the layers beneath.
//!
//! # Examples
//!
//! The [`examples`](https://github.com/Kato-emb/isobmff-rs/tree/main/examples)
//! directory of the repository holds one program per use, each written
//! against this crate and the I/O it reads and writes through, and run from a
//! checkout as `cargo run -p isobmff-examples --example <name> -- <arguments>`.

#![no_std]

// Why not `pub use isobmff_core as core`: rustdoc renders that as one line
// under Re-exports, with no module page and no `isobmff::core::…` items to
// search, and a flat glob would let the `Error` of one crate collide with the
// `Error` of another.
/// The framing of a box and the codecs its fields are read and written by —
/// the `isobmff-core` crate whole
pub mod core {
    pub use isobmff_core::*;
}

/// A file framed as the sequence of boxes it is — the `isobmff-sequence` crate
/// whole
pub mod sequence {
    pub use isobmff_sequence::*;
}

/// The catalog of ISO/IEC 14496-12 boxes — the `isobmff-boxes` crate whole
pub mod boxes {
    pub use isobmff_boxes::*;
}

/// Where the samples of a presentation lie, and the samples themselves — the
/// `isobmff-sample` crate whole
pub mod sample {
    pub use isobmff_sample::*;
}

/// The order the boxes of a file stand in, and the demux and mux FSMs that
/// read and write one — the `isobmff-structure` crate whole
pub mod structure {
    pub use isobmff_structure::*;
}

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
