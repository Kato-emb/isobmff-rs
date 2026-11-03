//! [`DemuxError`], [`MuxError`] and [`TimelineError`], the reasons the bytes of a file do not read as its boxes, boxes do not write as a file, or an edit list does not map a timeline

mod demux;
mod mux;
mod timeline;

pub use demux::DemuxError;
pub use mux::MuxError;
pub use timeline::TimelineError;
