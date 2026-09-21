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
//!    as media data, or passed over. Three structures are held: the
//!    fragmented movie file of Annex A.8 — the brands, the movie, then one
//!    movie fragment after another with the media data beside it — the
//!    non-fragmented movie file of §8.2.1 — the brands, the one movie, and
//!    the media data it declares, lying before the movie or after it — and
//!    the media segment of §8.16 — the brands, then the fragments and their
//!    media data, the movie they continue held apart from it. The structure
//!    is the only layer that knows how a file is put together, and the order
//!    a file breaks is its failure.
//! 6. **Stack.** [`FragmentedReader`], [`FragmentedWriter`],
//!    [`NonFragmentedReader`], [`NonFragmentedWriter`], [`MediaSegmentReader`]
//!    and [`MediaSegmentWriter`] wire layers 1 to 5 into one machine per
//!    structure and direction. A stack holds no rule and no failure kind of
//!    its own: it passes every value between the layers, so a caller hands
//!    over bytes and takes samples, or hands over samples and takes bytes,
//!    and never sees one. Every offset above the framing is a file offset —
//!    the extents the framing reports for a file handed over from its first
//!    byte, the chunk offsets and base data offsets the boxes declare
//!    (§8.7.5, §8.8.7) — and no layer here knows any other.
//! 7. **The I/O.** Where the bytes come from and go to is the caller's: the
//!    file is handed over from its first byte, output is taken, and what the
//!    reader says it still lacks is fetched or not, so a `File`, a socket, or a
//!    buffer already in memory drives the six layers above the same way. Where
//!    the file lies in its resource, and how a file offset becomes a seek or a
//!    range, is settled here and in none of them. The `std` feature holds one
//!    such driver per stack: a demuxer reads the file off a `Read + Seek` a
//!    cut at a time, seeks to what the reader lacks wherever the file passed
//!    it by, and yields the samples; a muxer writes what the writer makes of
//!    each call to a `Write`. What either cannot carry through — a source or
//!    a sink failing, or a layer beneath refusing the file — is
//!    [`DriverError`].
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
//! whole, a sample owns the bytes it carries, the extents a movie or a fragment
//! declares are held until the data that meets them arrives, the samples of a
//! fragment or a chunk being written are held until it is laid down, and the
//! movie a non-fragmented file is written against until the file is over. The
//! `std` feature, off by default, adds the demuxers and the muxers alone: the
//! six layers beneath them are the same with it or without.

#![no_std]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "std")]
mod driver;
#[cfg(feature = "std")]
mod driver_error;
mod fragmented_movie;
mod media_segment;
mod non_fragmented_movie;
mod structure_error;
mod whole_box;

#[cfg(feature = "std")]
pub use driver_error::{DriverError, DriverErrorKind};
#[cfg(feature = "std")]
pub use fragmented_movie::{FragmentedDemuxer, FragmentedMuxer};
pub use fragmented_movie::{FragmentedReader, FragmentedWriter};
#[cfg(feature = "std")]
pub use media_segment::{MediaSegmentDemuxer, MediaSegmentMuxer};
pub use media_segment::{MediaSegmentReader, MediaSegmentWriter};
#[cfg(feature = "std")]
pub use non_fragmented_movie::{NonFragmentedDemuxer, NonFragmentedMuxer};
pub use non_fragmented_movie::{NonFragmentedReader, NonFragmentedWriter};
pub use structure_error::{StructureError, StructureErrorKind};

#[cfg(feature = "std")]
pub(crate) use driver::{Demuxer, Muxer, PollOutput, ReadSamples};
pub(crate) use whole_box::{WholeBoxReader, compact_box_header, whole_box_header, whole_payload};

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
