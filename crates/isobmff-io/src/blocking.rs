//! The source and the sink over `std::io`
//!
//! [`Source`] reads a file at the offsets a demux FSM names off a source that
//! is `Read + Seek`; [`Sink`] writes the chunks a mux FSM made to a sink that
//! is `Write`.

mod sink;
mod source;

pub use sink::Sink;
pub use source::Source;
