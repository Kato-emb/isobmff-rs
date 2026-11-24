//! Where the bytes of the samples of a track lie, and what their data offsets count from
//!
//! The track data layout structures, ISO/IEC 14496-12 §8.7, give each sample
//! of a sample table the chunk it lies in (`stsc`, `stco` or `co64`), its
//! size (`stsz` or `stz2`), and the sample entry that names the data
//! reference entry specifying its container (`dref`). [`data_entry`] looks
//! that entry up. The data offset of a sample counts from a data origin,
//! §6.1 Table 1, which its location names; [`DataOrigins`] takes where the
//! data origins of a file lie, and turns each location into one at its file
//! offset.

mod chunk;
mod data_origin;
mod data_reference;

pub(super) use chunk::{
    ChunkSamples, check_data_offsets, check_sample_to_chunk, chunk_count, chunk_sample_count,
    chunk_samples, sample_count, sample_description_indices,
};
pub use data_origin::DataOrigins;
pub use data_reference::data_entry;
pub(super) use data_reference::sample_table_data_origin;
