//! [`DemuxError`] and [`MuxError`], the reasons the bytes of a file do not read as its boxes, or boxes do not write as a file

mod demux;
mod mux;

pub use demux::DemuxError;
pub use mux::MuxError;
