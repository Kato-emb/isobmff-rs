//! [`ChunkOffsetBox`] and [`ChunkLargeOffsetBox`], the variants that state the chunk offsets of a sample table

mod co64;
mod stco;

pub use co64::ChunkLargeOffsetBox;
pub use stco::ChunkOffsetBox;
