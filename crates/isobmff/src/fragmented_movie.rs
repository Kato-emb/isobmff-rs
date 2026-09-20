//! [`FragmentedReader`] and [`FragmentedWriter`], a fragmented movie file read and written through the layers this crate holds, ISO/IEC 14496-12 Annex A.8

mod reader;
mod writer;

pub use reader::FragmentedReader;
pub use writer::FragmentedWriter;
