//! [`NonFragmentedReader`] and [`NonFragmentedWriter`], a non-fragmented movie file read and written through the layers this crate holds, ISO/IEC 14496-12 §8.2.1 and §8.7

#[cfg(feature = "std")]
mod demuxer;
#[cfg(feature = "std")]
mod muxer;
mod reader;
mod structure;
mod writer;

#[cfg(feature = "std")]
pub use demuxer::NonFragmentedDemuxer;
#[cfg(feature = "std")]
pub use muxer::NonFragmentedMuxer;
pub use reader::NonFragmentedReader;
pub use writer::NonFragmentedWriter;

use structure::{NonFragmentedDisposition, NonFragmentedStructure};
