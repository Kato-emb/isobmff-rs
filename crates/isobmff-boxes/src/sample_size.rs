//! [`SampleSizeBox`] and [`CompactSampleSizeBox`], the variants that state the sample sizes of a sample table

mod stsz;
mod stz2;

pub use stsz::{SampleSizeBox, SampleSizes};
pub use stz2::CompactSampleSizeBox;
