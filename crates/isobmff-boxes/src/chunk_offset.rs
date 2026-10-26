//! [`ChunkOffsetBox`] and [`ChunkLargeOffsetBox`], the variants that state the chunk offsets of a sample table

mod co64;
mod stco;

use isobmff_core::{BoxDecode, BoxDefinition, BoxType, BoxVariants, Error, RawBox};

pub use co64::ChunkLargeOffsetBox;
pub use stco::ChunkOffsetBox;

/// Chunk offsets of a sample table, stated by one of the variants of ISO/IEC 14496-12 §8.7.5
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ChunkOffsetVariant {
    /// Offsets in 32 bits
    ChunkOffset(ChunkOffsetBox),
    /// Offsets in 64 bits
    ChunkLargeOffset(ChunkLargeOffsetBox),
}

impl BoxVariants for ChunkOffsetVariant {
    const VARIANTS: &'static [BoxType] = &[ChunkOffsetBox::BOX_TYPE, ChunkLargeOffsetBox::BOX_TYPE];

    fn decode_variant(child: RawBox<'_>) -> Result<Self, Error> {
        if child.header().box_type() == ChunkOffsetBox::BOX_TYPE {
            ChunkOffsetBox::decode_payload(child.payload()).map(Self::ChunkOffset)
        } else {
            ChunkLargeOffsetBox::decode_payload(child.payload()).map(Self::ChunkLargeOffset)
        }
    }
}
