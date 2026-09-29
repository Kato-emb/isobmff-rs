//! Drivers running the demux and mux FSMs of an ISO base media file over an I/O
//!
//! The FSMs of [`isobmff_structure`] reach for no source or sink of their
//! own: a demux FSM takes the file as bytes, and states the bytes it lacks
//! and where its input stands; a mux FSM takes boxes and samples, and hands
//! over the bytes it made of them. What this crate settles is where those
//! bytes come from and go to, and nothing else. A [`DemuxDriver`] reads the
//! file off a source that seeks, at the offset the FSM states — the bytes
//! [`wanted_extent`](Demux::wanted_extent) names, or else on from
//! [`input_offset`](Demux::input_offset) — and hands over the samples as they
//! come whole; a [`MuxDriver`] writes to a sink the bytes the FSM made. Each
//! drives any FSM through the traits [`Demux`] and [`Mux`], which the six
//! FSMs implement and which are sealed: no type outside this crate can
//! implement them. Neither driver holds anything the FSM already holds, and
//! the FSM is reached through the driver's `fsm` and `fsm_mut` for what it
//! read into values and for its own verbs.
//!
//! A demux FSM of a fragmented movie file or a media segment reads the file
//! from a place an index names as well as from its start: the indexes it
//! passes — a `sidx` (§8.16.3), and in a fragmented movie file an `mfra`
//! (§8.8.9) — are there to read as values, and its `resume_at`
//! ([`FragmentedDemuxFsm::resume_at`](isobmff_structure::FragmentedDemuxFsm::resume_at),
//! [`MediaSegmentDemuxFsm::resume_at`](isobmff_structure::MediaSegmentDemuxFsm::resume_at))
//! restarts the reading at the offset of the fragment the caller found a time
//! in, the driver seeking there as it reads on. Where to look and where to
//! resume are the caller's: a demux driver reads the end of the file for its
//! `mfra` when asked
//! ([`DemuxDriver::locate_movie_fragment_random_access`]), and never on its
//! own.
//!
//! # Asynchronous and blocking
//!
//! The crate root holds the drivers over `futures::io`, and [`blocking`] the
//! ones over `std::io`, of the same names and verbs: the asynchronous ones
//! `async fn`, and the blocking [`DemuxDriver`](blocking::DemuxDriver) an
//! `Iterator`. Every `async fn` of this crate is cancellation safe. A future
//! dropped part way leaves no request of its own behind: the call that
//! follows derives what to read from the FSM again, trusting where the source
//! stands only once a seek or a read there completed, and a
//! [`flush`](MuxDriver::flush) carries on the chunk the sink was taking from
//! the byte it stopped at — no byte read or written twice, and none lost. A
//! [`locate_movie_fragment_random_access`](DemuxDriver::locate_movie_fragment_random_access)
//! dropped part way is made again from its start. [`Error`] is what any of
//! them reports — a file that does not read or write failed at the source or
//! the sink, or at the FSM.
//!
//! # Driving an FSM without a driver
//!
//! A caller whose source cannot seek — a socket, a live stream of segments —
//! or is positioned by nature — a slice in memory, a blob, a range request —
//! drives the FSM itself, and needs no driver: it hands over what the FSM
//! asks for, reading bytes where [`wanted_extent`](Demux::wanted_extent)
//! names them, else on from [`input_offset`](Demux::input_offset), and
//! declares the file over where it ends. A caller whose source reads on in
//! order alone stops where a want names bytes it has passed.
//! `examples/drive_non_fragmented_demux_fsm.rs` drives one over a file this
//! way.
//!
//! ```
//! use isobmff_structure::NonFragmentedDemuxFsm;
//! # use isobmff_test_support::non_fragmented_file;
//! // A file whose movie lies after its media data, held in memory
//! let file = non_fragmented_file(&[&[b"SAMP", b"DATA"]], false);
//! let mut fsm = NonFragmentedDemuxFsm::new();
//! let mut read_back = Vec::new();
//!
//! let mut finished = false;
//! while !finished {
//!     // The bytes the FSM wants, else the file on from where its input stands, cut anywhere
//!     let input_offset = fsm.input_offset() as usize;
//!     if let Some(wanted) = fsm.wanted_extent() {
//!         let data = &file[wanted.start as usize..wanted.end as usize];
//!         fsm.handle_data(wanted.start, data)?;
//!     } else if let Some(input) = file.get(input_offset..).filter(|input| !input.is_empty()) {
//!         fsm.handle_input(&input[..input.len().min(7)])?;
//!     } else {
//!         fsm.finish()?;
//!         finished = true;
//!     }
//!
//!     // The samples it completed come out as they come whole
//!     while let Some(sample) = fsm.poll_sample() {
//!         read_back.push(sample.into_data());
//!     }
//! }
//! assert_eq!(read_back, [b"SAMP".to_vec(), b"DATA".to_vec()]);
//! # Ok::<(), isobmff_structure::Error>(())
//! ```
//!
//! # `std`
//!
//! This is the one crate of the workspace that needs `std`: a source, a sink
//! and a seek are what it is for. The six layers beneath it are `no_std`.

extern crate alloc;

mod driver;
mod error;
mod stack;

pub mod blocking;

pub use driver::{DemuxDriver, MuxDriver};
pub use error::{Error, ErrorKind};
pub use stack::{Demux, Mux};
