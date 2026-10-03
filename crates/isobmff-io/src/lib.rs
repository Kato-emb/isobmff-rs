//! A source read at an offset and a sink written a chunk at a time, for the demux and mux FSMs of an ISO base media file
//!
//! The FSMs of `isobmff-structure` reach for no source or sink of their
//! own: a demux FSM states the one read it wants next, as an offset and an
//! optional length, and takes the bytes read there with the offset they were
//! read at; a mux FSM takes boxes and samples, and hands over the chunks it
//! made of them. This crate knows no FSM. [`Source`] reads a file at an
//! offset off a source that seeks, and [`Sink`] writes chunks to a sink; the
//! loop between them and the FSM is the caller's, and is a few lines.
//!
//! ```
//! use std::io::Cursor;
//!
//! use isobmff_io::blocking::Source;
//! use isobmff_structure::NonFragmentedDemuxFsm;
//! # use isobmff_test_support::non_fragmented_file;
//! // A file whose movie lies after its media data
//! let mut source = Source::new(Cursor::new(non_fragmented_file(&[&[b"SAMP", b"DATA"]], false)))?;
//! let mut fsm = NonFragmentedDemuxFsm::new();
//! let mut read_back = Vec::new();
//!
//! while let Some(wanted) = fsm.wanted_input() {
//!     // The read the FSM wants, an empty one declaring the file over
//!     let bytes = source.read_at(wanted.offset(), wanted.length())?;
//!     let handed = if bytes.is_empty() { fsm.finish() } else { fsm.handle_input(wanted.offset(), bytes) };
//!
//!     // The samples it completed come out before any failure
//!     while let Some(sample) = fsm.poll_sample() {
//!         read_back.push(sample.into_data());
//!     }
//!     handed?;
//! }
//! assert_eq!(read_back, [b"SAMP".to_vec(), b"DATA".to_vec()]);
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! The samples a failing input completed come out before its failure is
//! acted on. Once the file is declared over or the FSM has failed it wants no
//! read, and the loop ends. A mux FSM is driven by its own verbs, and what it
//! made is written by `sink.write(core::iter::from_fn(|| fsm.poll_output()))`,
//! then `sink.flush()` where the bytes are to reach the medium.
//!
//! A demux FSM of a fragmented movie file or a media segment reads the file
//! from a place an index names as well as from its start: the indexes it
//! passes — a `sidx` (§8.16.3), and in a fragmented movie file an `mfra`
//! (§8.8.9) — are there to read as values, and its `resume_at` restarts the
//! reading at an offset an index names, the read it wants next lying there.
//! Where to look and where to resume are the caller's: a source reads the end
//! of the file for its `mfra` when asked
//! ([`Source::locate_movie_fragment_random_access`]), and never on its own.
//!
//! # Asynchronous and blocking
//!
//! The crate root holds the source and the sink over `futures::io`, and
//! [`blocking`] the ones over `std::io`, of the same names and verbs. Every
//! `async fn` of this crate is cancellation safe: a [`Source::read_at`]
//! dropped part way and made again reads the same bytes, trusting where the
//! source stands only once a seek or a read there completed; a
//! [`Sink::write`] dropped part way is carried on by the next from the byte
//! the sink stopped at, the chunks it did not take left with the FSM; a
//! [`Source::locate_movie_fragment_random_access`] dropped part way is made
//! again from its start. What fails is reported as `std::io` reports it; what
//! the FSM refuses is the FSM's own failure.
//!
//! # A source that does not seek
//!
//! A caller whose source cannot seek, or is positioned by nature, needs no
//! [`Source`]: how it drives a demux FSM is in the crate documentation of
//! `isobmff-structure`.
//!
//! # `std`
//!
//! This is the one crate of the workspace that needs `std`: a source, a sink
//! and a seek are what it is for. The six layers beneath it are `no_std`.

extern crate alloc;

mod sink;
mod source;
mod transfer;

pub mod blocking;

pub use sink::Sink;
pub use source::Source;
