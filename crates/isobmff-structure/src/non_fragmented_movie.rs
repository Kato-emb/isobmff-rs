//! [`NonFragmentedMuxFsm`], a non-fragmented movie file written through the layers this crate holds, ISO/IEC 14496-12 §8.2.1 and §8.7

mod mux;
mod structure;

pub use mux::NonFragmentedMuxFsm;

use structure::{NonFragmentedDisposition, NonFragmentedStructure};
