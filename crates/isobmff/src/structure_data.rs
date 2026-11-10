//! The structure-data of a file, resolved to where the bytes of each sample lie, and written from it
//!
//! [`StructureDataReader`] takes the `moov` of a file, and each `moof` after
//! it, read into their values, and returns the
//! [`SampleLocation`](crate::SampleLocation) of each sample they declare. [`SampleTableWriter`] takes the locations of the samples of
//! a track and writes the sample table that declares them.

mod error;
mod movie_fragment;
mod reader;
mod sample_table;
mod writer;

pub use error::Error;
pub use movie_fragment::movie_fragment_sample_count;
pub use reader::StructureDataReader;
pub use writer::SampleTableWriter;
