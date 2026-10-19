//! The order the top-level boxes of an ISO base media file stand in, ISO/IEC 14496-12 §6.2.3
//!
//! A presentation is carried as samples — ISO/IEC 14496-12 §3.1.14 has a sample
//! as all the data associated with a single timestamp. [`MovieDemuxer`](crate::MovieDemuxer)
//! takes a movie file as it arrives, fragmented or not, and reports the
//! [`Sample`](crate::sample::Sample)s it carries, and [`MediaSegmentDemuxer`](crate::MediaSegmentDemuxer)
//! does the same for a media segment delivered apart from the movie it
//! continues; [`FragmentedMuxer`](crate::FragmentedMuxer),
//! [`NonFragmentedMuxer`](crate::NonFragmentedMuxer) and [`MediaSegmentMuxer`](crate::MediaSegmentMuxer) go the other way, laying
//! samples down as a file or a segment of each kind. [`MovieDemuxer`](crate::MovieDemuxer)
//! gathering with [`SampleExtents`](crate::SampleExtents) reads a movie file the same way short of
//! gathering the samples, and reports the
//! [`SampleExtent`](crate::sample::SampleExtent) of each, where its bytes lie
//! in the file. None reaches for a source
//! or a sink of its own: when to read or write, and from or to where, stay with
//! the caller, through [the caller's loop](#the-callers-loop).
//!
//! # The layers a file is read through
//!
//! Seven layers stand between a file and the samples it carries, joined only
//! by the values that pass between them: a box event, a typed box, an extent,
//! a sample, a disposition. No layer but the demuxers and muxers of layer 6 holds
//! another's machine. Listed from the framing up to the caller:
//!
//! 1. **Framing.** A file is a sequence of objects, called boxes (§4.2), and
//!    framing that sequence is the work of
//!    [`BoxReader`](crate::sequence::BoxReader) and
//!    [`BoxWriter`](crate::sequence::BoxWriter), which
//!    [`sequence`](crate::sequence) holds. They read no box into a value:
//!    which boxes matter is not theirs to say.
//! 2. **Box values.** The events of one box gathered whole and read into its
//!    value, bounded by a limit on what the box may declare, and a value
//!    written whole into the events of its box. Held by the demuxers and the
//!    muxers, where a box event and [`BoxDecode`](crate::BoxDecode) first
//!    meet.
//! 3. **Sample resolution.** Where every sample of a presentation lies and what
//!    is true of it, resolved out of the boxes that declare it into a
//!    [`SampleExtent`](crate::sample::SampleExtent): the sample tables of a
//!    movie (§8.7) through
//!    [`sample_table::sample_extents`](crate::sample::sample_table::sample_extents),
//!    or a movie fragment against the movie it continues (§8.8) through
//!    [`movie_fragment::sample_extents`](crate::sample::movie_fragment::sample_extents).
//!    The writing side is the mirror:
//!    [`MovieFragmentWriter`](crate::sample::MovieFragmentWriter) lays samples
//!    out as a `moof` and the media data beside it, and
//!    [`SampleTableWriter`](crate::sample::SampleTableWriter) as the sample
//!    tables of a movie. [`sample`](crate::sample) holds this layer and the next.
//! 4. **Sample gathering.** [`SampleReader`](crate::sample::SampleReader)
//!    holds the extents it is handed and fills them out of the bytes that
//!    arrive, each piece with the offset it starts at; a
//!    [`Sample`](crate::sample::Sample) comes out once its bytes have, and the
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
//! 6. **Demuxers and muxers.** [`MovieDemuxer`](crate::MovieDemuxer), [`MediaSegmentDemuxer`](crate::MediaSegmentDemuxer),
//!    [`FragmentedMuxer`](crate::FragmentedMuxer), [`NonFragmentedMuxer`](crate::NonFragmentedMuxer) and [`MediaSegmentMuxer`](crate::MediaSegmentMuxer)
//!    wire layers 1 to 5 into machines: one demuxer reads a movie file,
//!    fragmented or not, through the structure of a fragmented one, and one
//!    reads a media segment; one muxer writes each kind.
//!    [`MovieDemuxer`](crate::MovieDemuxer) gathering with [`SampleExtents`](crate::SampleExtents) wires layers 1, 2, 3
//!    and 5 alone, and hands over the extents layer 3 resolves in place of
//!    samples. A demuxer holds
//!    one rule of its own: it keeps where the input it takes in order stands,
//!    and states the one read it wants next — an extent layer 4 lacks whose
//!    start that input has passed, else the continuation of that input —
//!    taking every input with the offset it was read at. Each passes every value between the layers,
//!    so a caller hands over bytes and takes samples (or extents, from a
//!    [`MovieDemuxer`](crate::MovieDemuxer) gathering with [`SampleExtents`](crate::SampleExtents)), or hands over
//!    samples and takes bytes, and
//!    never sees one. Every offset above the framing is
//!    a file offset — the extents the framing reports, counted from the first
//!    byte of the file or from the offset the demuxer last resumed at, the
//!    chunk offsets and base data offsets the boxes declare (§8.7.5, §8.8.7)
//!    — and no layer here knows any other.
//! 7. **The I/O.** Where the bytes come from and go to is the caller's: the
//!    file is handed over from its first byte, or from a resume point an index
//!    names, output is taken, and what the demuxer says it still lacks is
//!    fetched or not, so a `File`, a socket, or a buffer already in memory
//!    drives the six layers above the same way. Where
//!    the file lies in its resource, and how a file offset becomes a seek or a
//!    range, is settled here and in none of them. No crate of the workspace
//!    holds this layer: it is the caller's code, the loop between a machine
//!    and its I/O.
//!
//! # The caller's loop
//!
//! The loop a demuxer is read by asks it for the one read it wants, makes
//! that read, and hands the bytes over at the offset they were read at, no
//! byte declaring the file over; the samples come out before any failure of
//! the input that completed them is acted on, and once the file is declared
//! over or the demuxer has failed it wants no read:
//!
//! ```
//! use isobmff::MovieDemuxer;
//! # use isobmff_test_support::non_fragmented_file;
//! // A file whose movie lies after its media data, held in memory
//! let file = non_fragmented_file(&[&[b"SAMP", b"DATA"]], false);
//! let mut demuxer = MovieDemuxer::new();
//! let mut read_back = Vec::new();
//!
//! while let Some(wanted) = demuxer.wanted_input() {
//!     // The read the demuxer wants, cut at seven bytes where it names no length
//!     let start = (wanted.offset() as usize).min(file.len());
//!     let length = wanted.length().map_or(7, |length| length as usize);
//!     let bytes = &file[start..(start + length).min(file.len())];
//!     let handed = if bytes.is_empty() { demuxer.finish() } else { demuxer.handle_input(wanted.offset(), bytes) };
//!
//!     // The samples it completed come out as they come whole, before any failure
//!     while let Some(sample) = demuxer.poll_output() {
//!         read_back.push(sample.into_data());
//!     }
//!     handed?;
//! }
//! assert_eq!(read_back, [b"SAMP".to_vec(), b"DATA".to_vec()]);
//! # Ok::<(), isobmff::DemuxError>(())
//! ```
//!
//! A muxer is driven by its own verbs, and the bytes it made are taken with
//! its `poll_output` and written whole where the caller writes. Over
//! `std::io`, the two loops are these, the demux loop seeking before every
//! read:
//!
//! ```
//! use std::io::{Cursor, Read, Seek, SeekFrom, Write};
//!
//! use isobmff::boxes::{SampleFlags, TrackExtendsBox};
//! use isobmff::sample::{Sample, SampleProperties};
//! use isobmff::{FragmentedMuxer, MovieDemuxer};
//! # use isobmff_test_support::{file_type, fragmented_movie};
//! // A fragment of two samples laid down, the bytes of each event written whole
//! let mut file = Cursor::new(Vec::new());
//! let mut muxer = FragmentedMuxer::new();
//! muxer.handle_file_type(file_type())?;
//! muxer.handle_movie(fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO)))?;
//! muxer.begin_fragment(1)?;
//! muxer.handle_sample(Sample::new(
//!     SampleProperties {
//!         track_id: 1,
//!         decode_time: 0,
//!         sample_duration: 1_024,
//!         sample_composition_time_offset: 0,
//!         sample_flags: SampleFlags::ZERO,
//!         sample_description_index: 1,
//!     },
//!     b"SAMP".to_vec(),
//! ))?;
//! muxer.handle_sample(Sample::new(
//!     SampleProperties {
//!         track_id: 1,
//!         decode_time: 1_024,
//!         sample_duration: 1_024,
//!         sample_composition_time_offset: 0,
//!         sample_flags: SampleFlags::ZERO,
//!         sample_description_index: 1,
//!     },
//!     b"DATA".to_vec(),
//! ))?;
//! muxer.finish_fragment()?;
//! muxer.finish()?;
//! while let Some(bytes) = muxer.poll_output() {
//!     file.write_all(&bytes)?;
//! }
//!
//! // The file read back where the demuxer wants, into a buffer of the caller's
//! let mut demuxer = MovieDemuxer::new();
//! let mut buffer = vec![0; 1024 * 1024];
//! let mut read_back = Vec::new();
//! while let Some(wanted) = demuxer.wanted_input() {
//!     file.seek(SeekFrom::Start(wanted.offset()))?;
//!     let read = file.read(&mut buffer)?;
//!     let handed = if read == 0 { demuxer.finish() } else { demuxer.handle_input(wanted.offset(), &buffer[..read]) };
//!     while let Some(sample) = demuxer.poll_output() {
//!         read_back.push(sample.into_data());
//!     }
//!     handed?;
//! }
//! assert_eq!(read_back, [b"SAMP".to_vec(), b"DATA".to_vec()]);
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! An asynchronous caller writes the same loops, awaiting the seek, the read
//! and the write. The read the demuxer wants does not move until bytes are handed
//! over, so a demux iteration dropped part way is made again from its seek.
//! The bytes taken from `poll_output` are the caller's: those not written
//! whole when their write is dropped are the caller's to keep.
//!
//! A caller whose source cannot seek — a socket, a live stream of segments —
//! hands every cut it reads over at the offset the demuxer names while the length
//! of the read wanted is `None`, and stops where a length is named. One whose
//! source is positioned by nature — a slice in memory, a blob, a range request
//! — reads where the demuxer names, and as much as the length it names where it
//! names one.
//!
//! A caller that holds a whole presentation in memory needs none of the
//! machines: [`boxes`](crate::boxes) reads the boxes into values,
//! [`sample_table::sample_extents`](crate::sample::sample_table::sample_extents)
//! names where its samples lie, and their bytes are sliced from there.

mod error;
mod fragmented_movie;
mod media_segment;
mod non_fragmented_movie;

pub(crate) use error::Error;
pub(crate) use fragmented_movie::{FragmentedDisposition, FragmentedStructure};
pub(crate) use media_segment::{MediaSegmentDisposition, MediaSegmentStructure};
pub(crate) use non_fragmented_movie::{NonFragmentedDisposition, NonFragmentedStructure};
