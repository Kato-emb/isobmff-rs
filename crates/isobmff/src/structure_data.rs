//! Samples structure-data declares, resolved to where their media data lies
//!
//! Structure-data describes media data by reference (ISO/IEC 14496-12 A.3):
//! the `moov` declares the samples of each track in its sample tables (§8.5),
//! and each `moof` adds samples to them in track runs (§8.8). A
//! [`StructureDataReader`] takes these boxes as they come in the file and
//! resolves each sample to a [`SampleLocation`](crate::SampleLocation): the
//! values declared about it, and the byte range of its media data in file
//! offsets. The writers, the other way, declare samples from where their
//! media data lies: a [`SampleTableWriter`] fills the sample tables of the
//! movie with the location of each sample, and a [`MovieFragmentWriter`]
//! declares the samples of each movie fragment, and returns the location it
//! gives each in the `mdat` that follows the `moof`.
//!
//! A sample table declares samples wherever their media data lies: the
//! chunks of this movie lie out of decoding order, and the tables written
//! from their locations declare them where they are.
//!
//! ```
//! use isobmff::boxes::{FileTypeBox, MovieBox};
//! use isobmff::structure_data::{SampleTableWriter, StructureDataLimits, StructureDataReader};
//! use isobmff::BoxDecode;
//!
//! // The `moov` after the `ftyp`, its chunk 2 before its chunk 1 in the file
//! let file: &[u8] = include_bytes!("../tests/data/out_of_order.mp4");
//! let (_, after_ftyp) = FileTypeBox::decode(file)?;
//! let (movie, _) = MovieBox::decode(after_ftyp)?;
//! let limits = StructureDataLimits {
//!     sample_count: 1024,
//!     segment_index_count: 0,
//!     subsegment_count: 0,
//!     leniency_count: 0,
//!     identified_media_data_count: 0,
//!     unresolved_sample_count: 0,
//!     compressed_box_count: 65_536,
//! };
//! let resolve = |movie: &MovieBox| {
//!     StructureDataReader::new(limits)
//!         .handle_movie(movie.clone())
//!         .map(|locations| locations.into_decoding_order())
//! };
//! let locations = resolve(&movie)?;
//!
//! // Sample tables written from the locations declare the samples where they lie
//! let mut writer = SampleTableWriter::new(movie)?;
//! writer.handle_locations(&locations)?;
//! assert_eq!(resolve(writer.finish())?, locations);
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```

mod compressed;
mod file_structure;
mod movie_fragment;
mod random_access;
mod reader;
mod sample_description;
mod sample_table;
mod segment_index;
mod writer;

pub use file_structure::{FragmentedOptions, SegmentOptions};
pub use random_access::movie_fragment_random_access_start;
pub use reader::{SampleLocations, StructureDataLimits, StructureDataReader};
pub use sample_description::{
    DataReference, DataReferenceRefusal, data_reference_index, located_in_file,
};
pub use sample_table::chunk_offsets;
#[cfg(test)]
pub(crate) use sample_table::fixtures;
pub use segment_index::{SegmentIndex, Subsegment};
pub use writer::{MovieFragmentWriter, SampleTableWriter};
