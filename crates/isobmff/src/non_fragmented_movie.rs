//! [`NonFragmentedReader`], a non-fragmented movie file read through the layers this crate holds, ISO/IEC 14496-12 §8.1.1 and §8.7

mod reader;
mod structure;

pub use reader::NonFragmentedReader;

use structure::NonFragmentedStructure;
