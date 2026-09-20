//! Sans-IO readers and writers for an ISO base media file and the samples it carries
//!
//! A presentation is carried as samples — ISO/IEC 14496-12 §3.1.14 has a sample
//! as all the data associated with a single timestamp. [`FragmentedReader`]
//! takes a fragmented movie file as it arrives and reports the [`Sample`]s it
//! carries, and [`NonFragmentedReader`] does the same for a non-fragmented
//! one; [`FragmentedWriter`] goes the other way, laying samples down as a
//! fragmented movie file. None reaches for a source or a sink of its own: when
//! to read or write, and from or to where, stay with the caller.
//!
//! # The layers a file is read through
//!
//! Seven layers stand between a file and the samples it carries, joined only
//! by the values that pass between them: a box event, a typed box, an extent,
//! a sample, a disposition. No layer but the stack holds another's machine.
//! Listed from the framing up to the caller:
//!
//! 1. **Framing.** A file is a sequence of objects, called boxes (§4.2), and
//!    framing that sequence is the work of [`BoxReader`] and [`BoxWriter`],
//!    which `isobmff-sequence` holds. They read no box into a value: which
//!    boxes matter is not theirs to say.
//! 2. **Box values.** The events of one box gathered whole and read into its
//!    value, bounded by a limit on what the box may declare. A module of this
//!    crate, where a box event and [`BoxDecode`] first meet.
//! 3. **Sample resolution.** Where every sample of a presentation lies and what
//!    is true of it, resolved out of the boxes that declare it into a
//!    [`SampleExtent`]: the sample tables of a movie (§8.7) through
//!    [`sample_table::sample_extents`], or a movie fragment against the movie
//!    it continues (§8.8) through [`movie_fragment::sample_extents`]. The
//!    writing side is the mirror: [`MovieFragmentWriter`] lays samples out as
//!    a `moof` and the media data beside it, and [`SampleTableWriter`] as the
//!    sample tables of a movie. `isobmff-sample` holds this layer and the next.
//! 4. **Sample gathering.** [`SampleReader`] holds the extents it is handed and
//!    fills them out of the bytes that arrive, each piece with the offset it
//!    starts at; a [`Sample`] comes out once its bytes have, and the extent it
//!    still lacks is named for a caller that can seek to fetch it. Bytes no
//!    extent names are dropped.
//! 5. **Structure.** The order of the top-level boxes of one kind of file, and
//!    what is to be done with each: read into a value, offered to the samples
//!    as media data, or passed over. Two structures are held so far: the
//!    fragmented movie file of Annex A.8 — the brands, the movie, then one
//!    movie fragment after another with the media data beside it — and the
//!    non-fragmented movie file of §8.2.1 — the brands, the one movie, and
//!    the media data it declares, lying before the movie or after it. It is
//!    the only layer that knows how a file is put together, and the order a
//!    file breaks is its failure.
//! 6. **Stack.** [`FragmentedReader`], [`FragmentedWriter`] and
//!    [`NonFragmentedReader`] wire layers 1 to 5 into one machine per
//!    structure and direction. A stack holds no rule and no failure kind of
//!    its own: a reading stack adds the offset a caller hands over to the
//!    extents the framing reports, and either passes every value between the
//!    layers, so a caller hands over bytes and takes samples, or hands over
//!    samples and takes bytes, and never sees one.
//! 7. **The I/O.** Where the bytes come from and go to is the caller's: input
//!    is handed over with the offset it lies at, output is taken, and what the
//!    reader says it still lacks is fetched or not, so a `File`, a socket, or a
//!    buffer already in memory drives the six layers above the same way.
//!
//! A caller that holds a whole presentation in memory needs none of the
//! machines: [`boxes`] frames it, [`sample_table::sample_extents`] names where
//! its samples lie, and their bytes are sliced from there.
//!
//! # Everything in one place
//!
//! The crates this one is built on are re-exported whole, so a caller reaching
//! for any layer names `isobmff` alone: [`isobmff_core`] for the framing and
//! the field codecs, [`isobmff_boxes`] for the catalog of boxes,
//! [`isobmff_sample`] for the sample layers. Their names are re-exported as
//! they stand, so documentation written against any of them reads against this
//! one.
//!
//! The names that could not stand are [`Error`] and [`ErrorKind`], which
//! [`isobmff_core`] holds: the failures of the framing are re-exported as
//! [`SequenceError`] and [`SequenceErrorKind`], and the failures of the sample
//! layers are [`SampleError`] and of the layers this crate holds
//! [`StructureError`], both named apart at the source rather than shadowed.
//!
//! The sample entries other specifications define over ISO/IEC 14496-12 sit in
//! a module per specification — [`avc`] for ISO/IEC 14496-15, [`mp4`] for
//! ISO/IEC 14496-14 — each behind the Cargo feature of the same name, on by
//! default. A caller that wants the base specification alone turns the default
//! features off.
//!
//! # `no_std`
//!
//! The crate is `no_std` but needs `alloc`: a box read into a value is gathered
//! whole, a sample owns the bytes it carries, the extents of a fragment are held
//! until the data that meets them arrives, and the samples of a fragment being
//! written are held until it is closed.

#![no_std]

extern crate alloc;

mod disposition;
mod fragmented_movie;
mod non_fragmented_movie;
mod structure_error;
mod whole_box;

pub use fragmented_movie::{FragmentedReader, FragmentedWriter};
pub use non_fragmented_movie::NonFragmentedReader;
pub use structure_error::{StructureError, StructureErrorKind};

pub(crate) use disposition::Disposition;
pub(crate) use whole_box::{WholeBoxReader, whole_box_header, whole_payload};

pub use isobmff_boxes::*;
pub use isobmff_core::*;
pub use isobmff_sample::*;
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
