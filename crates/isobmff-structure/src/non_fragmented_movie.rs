//! [`NonFragmentedReader`] and [`NonFragmentedWriter`], a non-fragmented movie file read and written through the layers this crate holds, ISO/IEC 14496-12 §8.2.1 and §8.7

mod reader;
mod structure;
mod writer;

pub use reader::NonFragmentedReader;
pub use writer::NonFragmentedWriter;

use structure::{NonFragmentedDisposition, NonFragmentedStructure};
