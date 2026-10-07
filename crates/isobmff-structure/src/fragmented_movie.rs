//! [`FragmentedMuxFsm`], a fragmented movie file written through the layers this crate holds, ISO/IEC 14496-12 Annex A.8

mod mux;
mod structure;

pub use mux::FragmentedMuxFsm;

pub(crate) use structure::{FragmentedDisposition, FragmentedStructure};
