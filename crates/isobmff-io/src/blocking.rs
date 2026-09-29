//! The drivers over `std::io`
//!
//! [`DemuxDriver`] drives a demux FSM over a source that is `Read + Seek` and
//! yields the samples the file carries as `Iterator` items; [`MuxDriver`]
//! drives a mux FSM onto a sink that is `Write` and writes the bytes it made.
//! What each takes and refuses is its FSM's contract.

mod driver;

pub use driver::{DemuxDriver, MuxDriver};
