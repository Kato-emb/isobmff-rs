//! The structure-data of a file, resolved to where the bytes of each sample lie
//!
//! [`StructureDataReader`] takes the `moov` of a file, read into its value, and
//! returns the [`SampleLocation`](crate::SampleLocation) of each sample its
//! tracks declare.

mod error;
mod reader;
mod sample_table;

pub use error::Error;
pub use reader::StructureDataReader;
