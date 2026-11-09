//! The structure-data of a file, resolved to where the bytes of each sample lie, and written from it
//!
//! [`StructureDataReader`] takes the `moov` of a file, read into its value, and
//! returns the [`SampleLocation`](crate::SampleLocation) of each sample its
//! tracks declare. [`SampleTableWriter`] takes the locations of the samples of
//! a track and writes the sample table that declares them.

mod error;
mod reader;
mod sample_table;
mod writer;

pub use error::Error;
pub use reader::StructureDataReader;
pub use writer::SampleTableWriter;
