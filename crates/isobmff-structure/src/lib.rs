//! The order the boxes of an ISO base media file stand in, and the stacks that read and write one
//!
//! A presentation is carried as samples — ISO/IEC 14496-12 §3.1.14 has a sample
//! as all the data associated with a single timestamp. [`FragmentedReader`]
//! takes a fragmented movie file as it arrives and reports the
//! [`Sample`](isobmff_sample::Sample)s it carries, [`NonFragmentedReader`] does
//! the same for a non-fragmented one, and [`MediaSegmentReader`] for a media
//! segment delivered apart from the movie it continues; [`FragmentedWriter`],
//! [`NonFragmentedWriter`] and [`MediaSegmentWriter`] go the other way, laying
//! samples down as a file or a segment of each kind. None reaches for a source
//! or a sink of its own: when to read or write, and from or to where, stay with
//! the caller. A caller that has an I/O to hand drives any of them through the
//! demuxers and the muxers of `isobmff-io`.
//!
//! # The layers a file is read through
//!
//! Seven layers stand between a file and the samples it carries, joined only
//! by the values that pass between them: a box event, a typed box, an extent,
//! a sample, a disposition. No layer but the stack holds another's machine.
//! Listed from the framing up to the caller:
//!
//! 1. **Framing.** A file is a sequence of objects, called boxes (§4.2), and
//!    framing that sequence is the work of
//!    [`BoxReader`](isobmff_sequence::BoxReader) and
//!    [`BoxWriter`](isobmff_sequence::BoxWriter), which `isobmff-sequence`
//!    holds. They read no box into a value: which boxes matter is not theirs
//!    to say.
//! 2. **Box values.** The events of one box gathered whole and read into its
//!    value, bounded by a limit on what the box may declare. A module of this
//!    crate, where a box event and [`BoxDecode`](isobmff_core::BoxDecode)
//!    first meet.
//! 3. **Sample resolution.** Where every sample of a presentation lies and what
//!    is true of it, resolved out of the boxes that declare it into a
//!    [`SampleExtent`](isobmff_sample::SampleExtent): the sample tables of a
//!    movie (§8.7) through
//!    [`sample_table::sample_extents`](isobmff_sample::sample_table::sample_extents),
//!    or a movie fragment against the movie it continues (§8.8) through
//!    [`movie_fragment::sample_extents`](isobmff_sample::movie_fragment::sample_extents).
//!    The writing side is the mirror:
//!    [`MovieFragmentWriter`](isobmff_sample::MovieFragmentWriter) lays samples
//!    out as a `moof` and the media data beside it, and
//!    [`SampleTableWriter`](isobmff_sample::SampleTableWriter) as the sample
//!    tables of a movie. `isobmff-sample` holds this layer and the next.
//! 4. **Sample gathering.** [`SampleReader`](isobmff_sample::SampleReader)
//!    holds the extents it is handed and fills them out of the bytes that
//!    arrive, each piece with the offset it starts at; a
//!    [`Sample`](isobmff_sample::Sample) comes out once its bytes have, and the
//!    extent it still lacks is named for a caller that can seek to fetch it.
//!    Bytes no extent names are dropped.
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
//!    range, is settled here and in none of them. This crate holds no such
//!    driver: the demuxers and the muxers of `isobmff-io`, one of each per stack,
//!    are the layer.
//!
//! A caller that holds a whole presentation in memory needs none of the
//! machines: [`isobmff_boxes`] reads its boxes into values,
//! [`sample_table::sample_extents`](isobmff_sample::sample_table::sample_extents)
//! names where its samples lie, and their bytes are sliced from there.
//!
//! # `no_std`
//!
//! The crate is `no_std` but needs `alloc`: a box read into a value is gathered
//! whole, a sample owns the bytes it carries, the extents a movie or a fragment
//! declares are held until the data that meets them arrives, the samples of a
//! fragment or a chunk being written are held until it is laid down, and the
//! movie a non-fragmented file is written against until the file is over.

#![no_std]

extern crate alloc;

mod fragmented_movie;
mod media_segment;
mod non_fragmented_movie;
mod structure_error;
mod whole_box;

pub use fragmented_movie::{FragmentedReader, FragmentedWriter};
pub use media_segment::{MediaSegmentReader, MediaSegmentWriter};
pub use non_fragmented_movie::{NonFragmentedReader, NonFragmentedWriter};
pub use structure_error::{StructureError, StructureErrorKind};

pub(crate) use whole_box::{WholeBoxReader, compact_box_header, whole_box_header, whole_payload};
