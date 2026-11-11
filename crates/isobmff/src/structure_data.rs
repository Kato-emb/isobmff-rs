//! The structure-data of a file, resolved to where the bytes of each sample lie, and written from it
//!
//! [`StructureDataReader`] takes the `moov` of a file, and each `moof` after
//! it, read into their values, and returns the
//! [`SampleLocation`](crate::SampleLocation) of each sample they declare. [`SampleTableWriter`] takes the locations of the samples of
//! a track and writes the sample table that declares them, and
//! [`MovieFragmentWriter`] the samples of each fragment and writes the `moof`
//! that declares them. [`media_duration`] and [`composition_end`] say how
//! long the samples of a sample table last.

mod error;
mod mismatch;
mod movie_fragment;
mod movie_fragment_writer;
mod reader;
mod sample;
mod sample_table;
mod sample_table_writer;

pub use error::Error;
pub use mismatch::Mismatch;
pub use movie_fragment::movie_fragment_sample_count;
pub use movie_fragment_writer::MovieFragmentWriter;
pub use reader::StructureDataReader;
pub use sample::TrackSampleDescription;
pub use sample_table::{composition_end, media_duration};
pub use sample_table_writer::SampleTableWriter;
