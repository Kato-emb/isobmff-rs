//! Sans-IO readers and writers for an ISO base media file and the samples it carries
//!
//! A presentation is carried as samples — ISO/IEC 14496-12 §3.1.14 has a sample
//! as all the data associated with a single timestamp.
//! [`structure::FragmentedReader`] takes a fragmented movie file as it arrives
//! and reports the [`sample::Sample`]s it carries,
//! [`structure::NonFragmentedReader`] does the same for a non-fragmented one,
//! and [`structure::MediaSegmentReader`] for a media segment delivered apart
//! from the movie it continues; [`structure::FragmentedWriter`],
//! [`structure::NonFragmentedWriter`] and [`structure::MediaSegmentWriter`] go
//! the other way, laying samples down as a file or a segment of each kind.
//! None reaches for a source or a sink of its own: when to read or write, and
//! from or to where, stay with the caller. Where the caller has an I/O to
//! hand, the `io` feature adds `isobmff::io`, the demuxers and the muxers that
//! drive each of them over it.
//!
//! # One module per crate
//!
//! Every module here is one crate of the workspace re-exported whole, and this
//! root holds nothing else: [`structure::FragmentedReader`] and
//! `isobmff_structure::FragmentedReader` are the same type, so documentation
//! written against any of those crates reads against this one.
//!
//! | module | crate | of the seven layers |
//! |---|---|---|
//! | [`core`] | `isobmff-core` | what all of them are defined over |
//! | [`sequence`] | `isobmff-sequence` | layer 1 |
//! | [`boxes`] | `isobmff-boxes` | the catalog layer 2 reads a box into |
//! | [`sample`] | `isobmff-sample` | layers 3 and 4 |
//! | [`structure`] | `isobmff-structure` | layers 2, 5 and 6 |
//! | `io` | `isobmff-io` | layer 7 |
//! | [`avc`] | `isobmff-avc` | none: the sample entries of ISO/IEC 14496-15 |
//! | [`mp4`] | `isobmff-mp4` | none: the sample entries and descriptors of ISO/IEC 14496-14 |
//!
//! What each module holds is its own summary below; what the seven layers are,
//! and what passes between them, [`isobmff_structure`] describes. `avc` and
//! `mp4` are on by default and `io` is not, so a caller that wants the base
//! specification alone turns the default features off, and one that wants a
//! demuxer or a muxer asks for `io`.
//!
//! Each crate names the failures of its own layers `Error` and `ErrorKind`, and
//! carries the failures of the layers beneath through whole rather than
//! translating them, so [`structure::Error`] reaches [`sample::Error`] and
//! [`sequence::Error`] reaches [`core::Error`].
//!
//! # `no_std`
//!
//! The crate is `no_std` but needs `alloc`, for the reasons
//! [`isobmff_structure`] states of the layers beneath. The `io` feature, off by
//! default, adds `isobmff::io` alone, which needs `std`: the six layers beneath
//! it are the same with it or without.
//!
//! # Examples
//!
//! The [`examples`](https://github.com/Kato-emb/isobmff-rs/tree/main/examples)
//! directory of the repository holds one program per use, each written
//! against this crate alone with its `io` feature on, and run from a
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

/// The order the boxes of a file stand in, and the stacks that read and write
/// one — the `isobmff-structure` crate whole
pub mod structure {
    pub use isobmff_structure::*;
}

/// The demuxers and the muxers driving the readers and writers over an I/O,
/// over `futures::io` at the root and over `std::io` in `blocking` — the
/// `isobmff-io` crate whole, behind the `io` feature
#[cfg(feature = "io")]
pub mod io {
    pub use isobmff_io::*;
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
