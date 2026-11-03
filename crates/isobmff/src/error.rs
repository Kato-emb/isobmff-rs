//! [`DemuxError`], [`MuxError`], [`TimelineError`] and [`FastStartError`], the reasons the bytes of a file do not read as its boxes, boxes do not write as a file, an edit list does not map a timeline, or a movie file does not lay out again

mod demux;
mod fast_start;
mod mux;
mod timeline;

pub use demux::DemuxError;
pub use fast_start::FastStartError;
pub use mux::MuxError;
pub use timeline::TimelineError;
