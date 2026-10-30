//! Samples structure-data declares, resolved to where their media data lies
//!
//! Structure-data describes media data by reference (ISO/IEC 14496-12 A.3):
//! the `moov` declares the samples of each track in its sample tables (§8.5),
//! and each `moof` adds samples to them in track runs (§8.8). A
//! [`StructureDataReader`] takes these boxes as they come in the file and
//! resolves each sample to a [`SampleLocation`](crate::SampleLocation): the
//! values declared about it, and the byte range of its media data in file
//! offsets. A writer, the other way, declares the samples a muxer writes in
//! the sample tables of the movie, or in a movie fragment for each run of
//! them.

mod movie_fragment;
mod reader;
mod sample_description;
mod sample_table;
mod segment_index;
mod writer;

pub use reader::{SampleLocations, StructureDataLimits, StructureDataReader};
#[cfg(test)]
pub(crate) use sample_table::fixtures;
pub use segment_index::{SegmentIndex, Subsegment, movie_fragment_random_access_start};
pub(crate) use writer::{MovieFragmentWriter, SampleTableWriter, StructureDataWriter};
