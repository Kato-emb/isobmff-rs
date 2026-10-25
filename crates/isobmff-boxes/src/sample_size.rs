//! [`SampleSizeBox`] and [`CompactSampleSizeBox`], the variants that state the sample sizes of a sample table

mod stsz;
mod stz2;

use isobmff_core::{BoxDecode, BoxDefinition, BoxType, BoxVariants, Error, RawBox};

pub use stsz::{SampleSizeBox, SampleSizes};
pub use stz2::CompactSampleSizeBox;

/// Sample sizes of a sample table, stated by one of the variants of ISO/IEC 14496-12 §8.7.3
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum SampleSizeVariant {
    /// Sizes in 32 bits, or one size for every sample
    SampleSize(SampleSizeBox),
    /// Sizes in 4, 8 or 16 bits
    CompactSampleSize(CompactSampleSizeBox),
}

impl BoxVariants for SampleSizeVariant {
    const VARIANTS: &'static [BoxType] = &[SampleSizeBox::BOX_TYPE, CompactSampleSizeBox::BOX_TYPE];

    fn decode_variant(child: RawBox<'_>) -> Result<Self, Error> {
        if child.header().box_type() == SampleSizeBox::BOX_TYPE {
            SampleSizeBox::decode_payload(child.payload()).map(Self::SampleSize)
        } else {
            CompactSampleSizeBox::decode_payload(child.payload()).map(Self::CompactSampleSize)
        }
    }
}
