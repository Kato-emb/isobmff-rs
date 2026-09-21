//! [`FragmentedReader`] and [`FragmentedWriter`], a fragmented movie file read and written through the layers this crate holds, ISO/IEC 14496-12 Annex A.8

#[cfg(feature = "std")]
mod demuxer;
#[cfg(feature = "std")]
mod muxer;
mod reader;
mod structure;
mod writer;

#[cfg(feature = "std")]
pub use demuxer::FragmentedDemuxer;
#[cfg(feature = "std")]
pub use muxer::FragmentedMuxer;
pub use reader::FragmentedReader;
pub use writer::FragmentedWriter;

use structure::FragmentedStructure;
