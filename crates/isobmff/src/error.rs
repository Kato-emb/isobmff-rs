//! [`ReadError`], [`WriteError`] and [`TimelineError`], the reasons the bytes of a file do not read as its boxes, boxes do not write as a file, or an edit list does not map a timeline

mod read;
mod timeline;
mod write;

pub use read::ReadError;
pub use timeline::TimelineError;
pub use write::WriteError;
