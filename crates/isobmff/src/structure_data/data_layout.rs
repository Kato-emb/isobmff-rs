//! Where the bytes of the samples of a track lie, and what their data offsets count from
//!
//! The track data layout structures, ISO/IEC 14496-12 §8.7, give each sample
//! of a sample table its framing: the chunk it lies in (`stsc`, `stco` or
//! `co64`), its size (`stsz` or `stz2`), and the sample entry that names the
//! data reference entry specifying its container (`dref`). [`data_entry`]
//! looks that entry up.

mod data_origin;
mod data_reference;
mod framing;
mod table;

pub(super) use data_origin::sample_table_data_origin;
pub use data_reference::data_entry;
pub(super) use framing::{
    SampleFramings, check_sample_framings, check_sample_to_chunk, chunk_count, chunk_sample_count,
    sample_count, sample_framings,
};
