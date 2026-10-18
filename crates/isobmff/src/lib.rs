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
//! None reaches for a source or a sink of its own, and none does I/O: when to
//! read or write, and from or to where, stay with the caller.
//!
//! # The root and the modules
//!
//! The root is the `isobmff-core` crate re-exported whole — [`BoxType`],
//! [`FourCc`], [`BoxDecode`] and the rest of what all the layers are defined
//! over. A module the table places in another crate is that crate re-exported
//! whole: [`sample::Sample`] and `isobmff_sample::Sample` are the same type,
//! so documentation written against any of those crates reads against this
//! one. A module placed in this crate holds its layers itself.
//!
//! | path | crate | of the seven layers |
//! |---|---|---|
//! | the root | `isobmff-core` | what all of them are defined over |
//! | [`sequence`] | `isobmff-sequence` | layer 1 |
//! | [`boxes`] | `isobmff-boxes` | the catalog layer 2 reads a box into |
//! | [`sample`] | `isobmff-sample` | layers 3 and 4 |
//! | [`structure`] | this crate | layers 2, 5 and 6 |
//! | [`avc`] | `isobmff-avc` | none: the sample entries of ISO/IEC 14496-15 |
//! | [`mp4`] | `isobmff-mp4` | none: the sample entries and descriptors of ISO/IEC 14496-14 |
//!
//! Layer 7, the I/O, is the caller's code, and no module holds it: the loop
//! between a machine and its I/O is
//! [the one `structure` states](structure#the-callers-loop).
//! What each module holds is its own summary below; what the seven layers
//! are, and what passes between them, [`structure`] describes.
//! `avc` and `mp4` are on by default, so a caller that wants the base
//! specification alone turns the default features off.
//!
//! Where the root or a module has failures of its own, it names them `Error`,
//! and carries the failures of the layers beneath through whole rather than
//! translating them, so [`structure::Error`] reaches [`sample::Error`] and
//! [`sequence::Error`] reaches [`Error`].
//!
//! # `no_std`
//!
//! The crate is `no_std` but needs `alloc`, for the reasons
//! [`structure`](structure#alloc) states of its layers.
//!
//! # Examples
//!
//! The [`examples`](https://github.com/Kato-emb/isobmff-rs/tree/main/examples)
//! directory of the repository holds one program per use, each written
//! against this crate and the I/O it reads and writes through, and run from a
//! checkout as `cargo run -p isobmff-examples --example <name> -- <arguments>`.

#![no_std]

extern crate alloc;

pub use isobmff_core::*;

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

pub mod structure;

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
