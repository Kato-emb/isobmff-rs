//! [`ChunkOffsetBox`] (`stco`) and [`ChunkLargeOffsetBox`] (`co64`), the chunk
//! offset tables of ISO/IEC 14496-12 §8.7.5, and [`ChunkOffsets`], the one of
//! them a sample table holds

mod co64;
mod stco;

pub use co64::{ChunkLargeOffsetBox, ChunkLargeOffsetEntry};
pub use stco::{ChunkOffsetBox, ChunkOffsetEntry};

use alloc::vec::Vec;
use core::slice;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, BoxVariants, Error, RawBox};

/// The offsets of the chunks of a track, stated at one width or the other
///
/// ISO/IEC 14496-12 §8.7.5. The spec closes the set: the offsets are stated
/// either in a `stco`, in 32 bits, or in a `co64`, in 64 bits, and a sample
/// table holds exactly one of the two. Which one is held is a fact of the
/// bytes: a table read from a `co64` writes back as a `co64`, whether or not
/// its offsets would have fit a `stco`, and only
/// [`from_offsets`](Self::from_offsets) chooses.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ChunkOffsets {
    /// Table stating the offsets in 32 bits, `stco`
    Stco(ChunkOffsetBox),
    /// Table stating the offsets in 64 bits, `co64`
    Co64(ChunkLargeOffsetBox),
}

impl ChunkOffsets {
    /// Creates the table from the offset every chunk starts at, in chunk order
    ///
    /// The offsets are stated in 32 bits where every one of them fits, and in
    /// 64 bits where any one of them does not; no offset is refused.
    ///
    /// # Examples
    ///
    /// ```
    /// use isobmff_boxes::{
    ///     ChunkLargeOffsetBox, ChunkLargeOffsetEntry, ChunkOffsetBox, ChunkOffsetEntry,
    ///     ChunkOffsets,
    /// };
    ///
    /// // Chunks that all start where 32 bits reach are stated in a `stco`
    /// assert_eq!(
    ///     ChunkOffsets::from_offsets([1_000, 2_000]),
    ///     ChunkOffsets::Stco(ChunkOffsetBox::new(vec![
    ///         ChunkOffsetEntry::new(1_000),
    ///         ChunkOffsetEntry::new(2_000),
    ///     ]))
    /// );
    ///
    /// // One chunk starting past them moves every offset to a `co64`
    /// assert_eq!(
    ///     ChunkOffsets::from_offsets([1_000, 1 << 32]),
    ///     ChunkOffsets::Co64(ChunkLargeOffsetBox::new(vec![
    ///         ChunkLargeOffsetEntry::new(1_000),
    ///         ChunkLargeOffsetEntry::new(1 << 32),
    ///     ]))
    /// );
    /// ```
    #[must_use]
    pub fn from_offsets(offsets: impl IntoIterator<Item = u64>) -> Self {
        let large: Vec<ChunkLargeOffsetEntry> = offsets
            .into_iter()
            .map(ChunkLargeOffsetEntry::new)
            .collect();
        let compact: Result<Vec<ChunkOffsetEntry>, _> = large
            .iter()
            .map(|entry| u32::try_from(entry.chunk_offset()).map(ChunkOffsetEntry::new))
            .collect();

        match compact {
            Ok(entries) => Self::Stco(ChunkOffsetBox::new(entries)),
            Err(_past_32_bits) => Self::Co64(ChunkLargeOffsetBox::new(large)),
        }
    }

    /// Returns the offset every chunk starts at in turn, however wide the table states it
    pub fn offsets(&self) -> impl Iterator<Item = u64> + '_ {
        match self {
            Self::Stco(stco) => Offsets::Stco(stco.entries().iter()),
            Self::Co64(co64) => Offsets::Co64(co64.entries().iter()),
        }
    }

    /// Returns the length this table occupies, header and payload
    pub(crate) fn encoded_len(&self) -> u64 {
        match self {
            Self::Stco(stco) => stco.encoded_len(),
            Self::Co64(co64) => co64.encoded_len(),
        }
    }

    /// Writes the table into the front of `buffer` and returns what is left
    pub(crate) fn encode<'buffer>(
        &self,
        buffer: &'buffer mut [u8],
    ) -> Result<&'buffer mut [u8], Error> {
        match self {
            Self::Stco(stco) => stco.encode(buffer),
            Self::Co64(co64) => co64.encode(buffer),
        }
    }
}

impl BoxVariants for ChunkOffsets {
    const VARIANTS: &'static [BoxType] = &[ChunkOffsetBox::BOX_TYPE, ChunkLargeOffsetBox::BOX_TYPE];

    fn decode_variant(child: RawBox<'_>) -> Result<Self, Error> {
        let box_type = child.header().box_type();
        let payload = child.payload();

        if box_type == ChunkOffsetBox::BOX_TYPE {
            ChunkOffsetBox::decode_payload(payload).map(Self::Stco)
        } else {
            // Why not a type check here too: `decode_variant` leaves routing a child
            // of one of `VARIANTS` to its caller, so the other of the two is what is left,
            // and a check would state a failure no correctly routed call can reach.
            ChunkLargeOffsetBox::decode_payload(payload).map(Self::Co64)
        }
    }
}

/// The offsets of the chunks of a track read one after another, however wide they are stated
enum Offsets<'entries> {
    Stco(slice::Iter<'entries, ChunkOffsetEntry>),
    Co64(slice::Iter<'entries, ChunkLargeOffsetEntry>),
}

impl Iterator for Offsets<'_> {
    type Item = u64;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Stco(entries) => entries.next().map(|entry| u64::from(entry.chunk_offset())),
            Self::Co64(entries) => entries.next().map(ChunkLargeOffsetEntry::chunk_offset),
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxVariants as _, boxes};

    use super::{
        ChunkLargeOffsetBox, ChunkLargeOffsetEntry, ChunkOffsetBox, ChunkOffsetEntry, ChunkOffsets,
    };

    /// Writes the table whole, header and payload, and reads it back as the slot does
    fn round_trip(chunk_offsets: &ChunkOffsets) -> ChunkOffsets {
        let mut buffer = vec![0; usize::try_from(chunk_offsets.encoded_len()).unwrap()];
        chunk_offsets.encode(&mut buffer).unwrap();

        ChunkOffsets::decode_variant(boxes(&buffer).next().unwrap().unwrap()).unwrap()
    }

    #[test]
    fn offsets_that_all_fit_32_bits_are_stated_in_a_stco() {
        assert_eq!(
            ChunkOffsets::from_offsets([]),
            ChunkOffsets::Stco(ChunkOffsetBox::new(Vec::new()))
        );
        assert_eq!(
            ChunkOffsets::from_offsets([1_000, u64::from(u32::MAX)]),
            ChunkOffsets::Stco(ChunkOffsetBox::new(vec![
                ChunkOffsetEntry::new(1_000),
                ChunkOffsetEntry::new(u32::MAX),
            ]))
        );
    }

    #[test]
    fn one_offset_past_32_bits_states_every_offset_in_a_co64() {
        assert_eq!(
            ChunkOffsets::from_offsets([1_000, u64::from(u32::MAX) + 1]),
            ChunkOffsets::Co64(ChunkLargeOffsetBox::new(vec![
                ChunkLargeOffsetEntry::new(1_000),
                ChunkLargeOffsetEntry::new(u64::from(u32::MAX) + 1),
            ]))
        );
    }

    #[test]
    fn the_offsets_come_out_as_they_went_in_however_wide_the_table_states_them() {
        for offsets in [vec![], vec![1_000, 2_000], vec![1_000, 1 << 32]] {
            assert_eq!(
                ChunkOffsets::from_offsets(offsets.iter().copied())
                    .offsets()
                    .collect::<Vec<_>>(),
                offsets
            );
        }
    }

    #[test]
    fn a_co64_stating_offsets_a_stco_would_reach_reads_back_as_a_co64() {
        let chunk_offsets =
            ChunkOffsets::Co64(ChunkLargeOffsetBox::new(vec![ChunkLargeOffsetEntry::new(
                1_000,
            )]));

        assert_eq!(round_trip(&chunk_offsets), chunk_offsets);
    }

    #[test]
    fn a_stco_reads_back_through_the_slot_as_a_stco() {
        let chunk_offsets = ChunkOffsets::from_offsets([1_000]);

        assert_eq!(round_trip(&chunk_offsets), chunk_offsets);
    }
}
