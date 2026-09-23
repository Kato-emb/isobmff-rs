//! [`SampleSizeBox`] (`stsz`) and [`CompactSampleSizeBox`] (`stz2`), the sample
//! size tables of ISO/IEC 14496-12 §8.7.3, and [`SampleSizes`], the one of them
//! a sample table holds

mod stsz;
mod stz2;

pub use stsz::{SampleSizeBox, SampleSizeEntries, SampleSizeEntry};
pub use stz2::{CompactSampleSizeBox, CompactSampleSizeEntry, FieldSize};

use core::slice;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, BoxVariants, Error, RawBox};

/// The sizes of the samples of a track, stated in one table or the other
///
/// ISO/IEC 14496-12 §8.7.3. The spec closes the set: the sizes are stated
/// either in a `stsz`, in 32 bits or as one size every sample shares, or in a
/// `stz2`, in 4, 8 or 16 bits, and a sample table holds exactly one of the two.
/// Which one is held is a fact of the bytes: a table read from a `stz2` writes
/// back as a `stz2`.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum SampleSizes {
    /// Table stating the sizes in 32 bits, `stsz`
    Stsz(SampleSizeBox),
    /// Table stating the sizes in fewer bits, `stz2`
    Stz2(CompactSampleSizeBox),
}

impl SampleSizes {
    /// Returns the size of every sample in turn, however the table states them
    pub fn sizes(&self) -> impl Iterator<Item = u32> + '_ {
        match self {
            Self::Stsz(stsz) => Sizes::Stsz(stsz.sizes()),
            Self::Stz2(stz2) => Sizes::Stz2(stz2.entries().iter()),
        }
    }

    /// Returns the length this table occupies, header and payload
    pub(crate) fn encoded_len(&self) -> u64 {
        match self {
            Self::Stsz(stsz) => stsz.encoded_len(),
            Self::Stz2(stz2) => stz2.encoded_len(),
        }
    }

    /// Writes the table into the front of `buffer` and returns what is left
    pub(crate) fn encode<'buffer>(
        &self,
        buffer: &'buffer mut [u8],
    ) -> Result<&'buffer mut [u8], Error> {
        match self {
            Self::Stsz(stsz) => stsz.encode(buffer),
            Self::Stz2(stz2) => stz2.encode(buffer),
        }
    }
}

impl BoxVariants for SampleSizes {
    const VARIANTS: &'static [BoxType] = &[SampleSizeBox::BOX_TYPE, CompactSampleSizeBox::BOX_TYPE];

    fn decode_variant(child: RawBox<'_>) -> Result<Self, Error> {
        let box_type = child.header().box_type();
        let payload = child.payload();

        if box_type == SampleSizeBox::BOX_TYPE {
            SampleSizeBox::decode_payload(payload).map(Self::Stsz)
        } else {
            // Why not a type check here too: `decode_variant` leaves routing a child
            // of one of `VARIANTS` to its caller, so the other of the two is what is left,
            // and a check would state a failure no correctly routed call can reach.
            CompactSampleSizeBox::decode_payload(payload).map(Self::Stz2)
        }
    }
}

/// The sizes of the samples of a track read one after another, however the table states them
enum Sizes<'entries, StszSizes> {
    Stsz(StszSizes),
    Stz2(slice::Iter<'entries, CompactSampleSizeEntry>),
}

impl<StszSizes: Iterator<Item = u32>> Iterator for Sizes<'_, StszSizes> {
    type Item = u32;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Stsz(sizes) => sizes.next(),
            Self::Stz2(entries) => entries.next().map(|entry| u32::from(entry.entry_size())),
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::{CompactSampleSizeBox, SampleSizeBox, SampleSizes};

    #[test]
    fn the_sizes_come_out_in_turn_whichever_table_states_them() {
        let stsz = SampleSizes::Stsz(SampleSizeBox::from_sizes([1_024, 512, 512]));
        let stz2 = SampleSizes::Stz2(CompactSampleSizeBox::from_sizes([300, 7, 7]));

        assert_eq!(stsz.sizes().collect::<Vec<_>>(), [1_024, 512, 512]);
        assert_eq!(stz2.sizes().collect::<Vec<_>>(), [300, 7, 7]);
    }
}
