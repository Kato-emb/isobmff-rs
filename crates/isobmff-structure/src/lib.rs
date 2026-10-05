//! The order the boxes of an ISO base media file stand in, and the demux and mux FSMs that read and write one
//!
//! A presentation is carried as samples — ISO/IEC 14496-12 §3.1.14 has a sample
//! as all the data associated with a single timestamp. [`FragmentedDemuxFsm`]
//! takes a fragmented movie file as it arrives and reports the
//! [`Sample`](isobmff_sample::Sample)s it carries, [`NonFragmentedDemuxFsm`] does
//! the same for a non-fragmented one, and [`MediaSegmentDemuxFsm`] for a media
//! segment delivered apart from the movie it continues; [`FragmentedMuxFsm`],
//! [`NonFragmentedMuxFsm`] and [`MediaSegmentMuxFsm`] go the other way, laying
//! samples down as a file or a segment of each kind. None reaches for a source
//! or a sink of its own: when to read or write, and from or to where, stay with
//! the caller, through [the caller's loop](#the-callers-loop).
//!
//! # The layers a file is read through
//!
//! Seven layers stand between a file and the samples it carries, joined only
//! by the values that pass between them: a box event, a typed box, an extent,
//! a sample, a disposition. No layer but the demux and mux FSMs holds
//! another's machine. Listed from the framing up to the caller:
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
//!    fragmented movie file of Annex A.8 — the brands, the movie, which may
//!    declare samples of its own, then one movie fragment after another, the
//!    media data lying anywhere among them — the
//!    non-fragmented movie file of §8.2.1 — the brands, the one movie, and
//!    the media data it declares, lying before the movie or after it — and
//!    the media segment of §8.16 — the brands, then the fragments and their
//!    media data, the movie they continue held apart from it, segments
//!    concatenated into one stream read as one. The structure is the only
//!    layer that knows how a file is put together, and the order a file
//!    breaks is its failure.
//! 6. **Demux and mux FSMs.** [`FragmentedDemuxFsm`], [`FragmentedMuxFsm`],
//!    [`NonFragmentedDemuxFsm`], [`NonFragmentedMuxFsm`], [`MediaSegmentDemuxFsm`]
//!    and [`MediaSegmentMuxFsm`] wire layers 1 to 5 into one machine per
//!    structure and direction. A demux FSM holds one rule of its own: it
//!    keeps where the input it takes in order stands, and states the one read
//!    it wants next — an extent layer 4 lacks whose start that input has
//!    passed, else the continuation of that input — taking every input with
//!    the offset it was read at. Each passes every value between the layers,
//!    so a caller hands over bytes and takes samples, or hands over samples
//!    and takes bytes, and never sees one. Every offset above the framing is
//!    a file offset — the extents the framing reports, counted from the first
//!    byte of the file or from the offset the demux FSM last resumed at, the
//!    chunk offsets and base data offsets the boxes declare (§8.7.5, §8.8.7)
//!    — and no layer here knows any other.
//! 7. **The I/O.** Where the bytes come from and go to is the caller's: the
//!    file is handed over from its first byte, or from a resume point an index
//!    names, output is taken, and what the demux FSM says it still lacks is
//!    fetched or not, so a `File`, a socket, or a buffer already in memory
//!    drives the six layers above the same way. Where
//!    the file lies in its resource, and how a file offset becomes a seek or a
//!    range, is settled here and in none of them. No crate of the workspace
//!    holds this layer: it is the caller's code, the loop between a machine
//!    and its I/O.
//!
//! # The caller's loop
//!
//! The loop a demux FSM is read by asks it for the one read it wants, makes
//! that read, and hands the bytes over at the offset they were read at, no
//! byte declaring the file over; the samples come out before any failure of
//! the input that completed them is acted on, and once the file is declared
//! over or the FSM has failed it wants no read:
//!
//! ```
//! use isobmff_structure::NonFragmentedDemuxFsm;
//! # use isobmff_test_support::non_fragmented_file;
//! // A file whose movie lies after its media data, held in memory
//! let file = non_fragmented_file(&[&[b"SAMP", b"DATA"]], false);
//! let mut fsm = NonFragmentedDemuxFsm::new();
//! let mut read_back = Vec::new();
//!
//! while let Some(wanted) = fsm.wanted_input() {
//!     // The read the FSM wants, cut at seven bytes where it names no length
//!     let start = (wanted.offset() as usize).min(file.len());
//!     let length = wanted.length().map_or(7, |length| length as usize);
//!     let bytes = &file[start..(start + length).min(file.len())];
//!     let handed = if bytes.is_empty() { fsm.finish() } else { fsm.handle_input(wanted.offset(), bytes) };
//!
//!     // The samples it completed come out as they come whole, before any failure
//!     while let Some(sample) = fsm.poll_sample() {
//!         read_back.push(sample.into_data());
//!     }
//!     handed?;
//! }
//! assert_eq!(read_back, [b"SAMP".to_vec(), b"DATA".to_vec()]);
//! # Ok::<(), isobmff_structure::Error>(())
//! ```
//!
//! A mux FSM is driven by its own verbs, and each chunk it made is taken with
//! its `poll_output` and written whole where the caller writes. Over
//! `std::io`, the two loops are these, the demux loop seeking before every
//! read:
//!
//! ```
//! use std::io::{Cursor, Read, Seek, SeekFrom, Write};
//!
//! use isobmff_boxes::{SampleFlags, TrackExtendsBox};
//! use isobmff_sample::Sample;
//! use isobmff_structure::{FragmentedDemuxFsm, FragmentedMuxFsm};
//! # use isobmff_test_support::{file_type, fragmented_movie};
//! // A fragment of two samples laid down, each chunk written whole
//! let mut file = Cursor::new(Vec::new());
//! let mut mux_fsm = FragmentedMuxFsm::new();
//! mux_fsm.handle_file_type(file_type())?;
//! mux_fsm.handle_movie(fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO)))?;
//! mux_fsm.begin_fragment(1)?;
//! mux_fsm.handle_sample(Sample::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec()))?;
//! mux_fsm.handle_sample(Sample::new(1, 1_024, 1_024, 0, SampleFlags::ZERO, 1, b"DATA".to_vec()))?;
//! mux_fsm.finish_fragment()?;
//! mux_fsm.finish()?;
//! while let Some(chunk) = mux_fsm.poll_output() {
//!     file.write_all(&chunk)?;
//! }
//!
//! // The file read back where the demux FSM wants, into a buffer of the caller's
//! let mut demux_fsm = FragmentedDemuxFsm::new();
//! let mut buffer = vec![0; 1024 * 1024];
//! let mut read_back = Vec::new();
//! while let Some(wanted) = demux_fsm.wanted_input() {
//!     file.seek(SeekFrom::Start(wanted.offset()))?;
//!     let read = file.read(&mut buffer)?;
//!     let handed = if read == 0 { demux_fsm.finish() } else { demux_fsm.handle_input(wanted.offset(), &buffer[..read]) };
//!     while let Some(sample) = demux_fsm.poll_sample() {
//!         read_back.push(sample.into_data());
//!     }
//!     handed?;
//! }
//! assert_eq!(read_back, [b"SAMP".to_vec(), b"DATA".to_vec()]);
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! An asynchronous caller writes the same loops, awaiting the seek, the read
//! and the write. The read the FSM wants does not move until bytes are handed
//! over, so a demux iteration dropped part way is made again from its seek. A chunk taken from `poll_output` is the caller's: one not written
//! whole when its write is dropped is the caller's to keep.
//!
//! A caller whose source cannot seek — a socket, a live stream of segments —
//! hands every cut it reads over at the offset the FSM names while the length
//! of the read wanted is `None`, and stops where a length is named. One whose
//! source is positioned by nature — a slice in memory, a blob, a range request
//! — reads where the FSM names, and as much as the length it names where it
//! names one.
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

mod demux_limits;
mod error;
mod fragmented_movie;
mod input_position;
mod media_segment;
mod non_fragmented_movie;
mod whole_box;

pub use demux_limits::DemuxLimits;
pub use error::{Error, ErrorKind};
pub use fragmented_movie::{FragmentedDemuxFsm, FragmentedMuxFsm};
pub use input_position::WantedInput;
pub use media_segment::{MediaSegmentDemuxFsm, MediaSegmentMuxFsm};
pub use non_fragmented_movie::{NonFragmentedDemuxFsm, NonFragmentedMuxFsm};

pub(crate) use input_position::{InputPosition, InputRoute};
pub(crate) use whole_box::{WholeBoxReader, compact_box_header, whole_box_header, whole_payload};
