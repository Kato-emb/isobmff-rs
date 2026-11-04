//! [`ReadError`], [`WriteError`], [`TimelineError`] and [`FastStartError`], the reasons the bytes of a file do not read as its boxes, boxes do not write as a file, an edit list does not map a timeline, or a movie file does not lay out again

mod fast_start;
mod read;
mod timeline;
mod write;

pub use fast_start::FastStartError;
pub use read::ReadError;
pub use timeline::TimelineError;
pub use write::WriteError;
