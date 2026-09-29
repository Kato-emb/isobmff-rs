//! [`NonFragmentedDemuxFsm`] and [`NonFragmentedMuxFsm`], a non-fragmented movie file read and written through the layers this crate holds, ISO/IEC 14496-12 §8.2.1 and §8.7

mod demux;
mod mux;
mod structure;

pub use demux::NonFragmentedDemuxFsm;
pub use mux::NonFragmentedMuxFsm;

use structure::{NonFragmentedDisposition, NonFragmentedStructure};
