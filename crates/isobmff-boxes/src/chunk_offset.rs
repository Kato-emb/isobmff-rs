//! [`ChunkOffsetBox`] (`stco`) and [`ChunkLargeOffsetBox`] (`co64`), the chunk
//! offset tables of ISO/IEC 14496-12 §8.7.5, and [`ChunkOffsets`], the one of
//! them a sample table holds

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, BoxVariants, Error, FieldReader, FieldWidth,
    FieldWriter, FullBoxFields, FullBoxFlags, RawBox,
};

/// Length of the fields that precede the entries
const FIXED_FIELDS_LEN: u64 = 8;

/// Length of one entry of a table stating its offsets in 32 bits
const ENTRY_LEN: u64 = 4;

/// Length of one entry of a table stating its offsets in 64 bits
const LARGE_ENTRY_LEN: u64 = 8;

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
            .map(|entry| u32::try_from(entry.chunk_offset).map(ChunkOffsetEntry::new))
            .collect();

        match compact {
            Ok(entries) => Self::Stco(ChunkOffsetBox::new(entries)),
            Err(_past_32_bits) => Self::Co64(ChunkLargeOffsetBox::new(large)),
        }
    }

    /// Returns the offset every chunk starts at in turn, however wide the table states it
    pub fn offsets(&self) -> impl Iterator<Item = u64> + '_ {
        match self {
            Self::Stco(stco) => Offsets::Stco(stco.entries.iter()),
            Self::Co64(co64) => Offsets::Co64(co64.entries.iter()),
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
    Stco(core::slice::Iter<'entries, ChunkOffsetEntry>),
    Co64(core::slice::Iter<'entries, ChunkLargeOffsetEntry>),
}

impl Iterator for Offsets<'_> {
    type Item = u64;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Stco(entries) => entries.next().map(|entry| u64::from(entry.chunk_offset)),
            Self::Co64(entries) => entries.next().map(|entry| entry.chunk_offset),
        }
    }
}

/// One entry of the table a [`ChunkOffsetBox`] holds
///
/// The offset reaches into the file that holds the media data, not into a box
/// of it, so building a file with its `moov` at the front means every one of
/// these depends on how long that `moov` turns out to be.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ChunkOffsetEntry {
    chunk_offset: u32,
}

impl ChunkOffsetEntry {
    /// Creates the entry from the offset its chunk starts at
    #[must_use]
    pub const fn new(chunk_offset: u32) -> Self {
        Self { chunk_offset }
    }

    /// Returns the offset into the file the chunk starts at
    #[must_use]
    pub const fn chunk_offset(&self) -> u32 {
        self.chunk_offset
    }
}

/// Box that states where every chunk of a track lies, in 32 bits
///
/// [`ChunkOffsetBox`] (`stco`), ISO/IEC 14496-12 §8.7.5. One entry per chunk,
/// in chunk order, which the `stsc` maps the samples onto. A presentation whose
/// chunks start past what 32 bits reach states them in a
/// [`ChunkLargeOffsetBox`] instead.
///
/// The `entry_count` field is not held: it counts the entries, so it is derived
/// on the way out. On the way in a count that disagrees with the entries fails
/// the box.
#[doc(alias = "stco")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ChunkOffsetBox {
    entries: Vec<ChunkOffsetEntry>,
}

impl ChunkOffsetBox {
    /// Creates the box from the entries it locates the chunks with
    #[must_use]
    pub const fn new(entries: Vec<ChunkOffsetEntry>) -> Self {
        Self { entries }
    }

    /// Returns the entries, in chunk order
    #[must_use]
    pub fn entries(&self) -> &[ChunkOffsetEntry] {
        &self.entries
    }
}

impl BoxDefinition for ChunkOffsetBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"stco");
}

impl BoxDecode for ChunkOffsetBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
    ///   ends inside a field of the box or inside one of its entries.
    /// * [`EntryCountMismatch`](isobmff_core::ErrorKind::EntryCountMismatch): the
    ///   `entry_count` field disagrees with the entries that follow it.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version != 0 {
            return Err(Error::unsupported_version(version));
        }

        let declared = u64::from(reader.read_u32()?);

        let mut entries = Vec::new();
        while !reader.remainder().is_empty() {
            entries.push(ChunkOffsetEntry {
                chunk_offset: reader.read_u32()?,
            });
        }

        let actual = entries.len() as u64;
        if actual != declared {
            return Err(Error::entry_count_mismatch(declared, actual));
        }

        Ok(Self { entries })
    }
}

impl BoxEncode for ChunkOffsetBox {
    fn payload_len(&self) -> u64 {
        let entries = (self.entries.len() as u64).saturating_mul(ENTRY_LEN);

        FIXED_FIELDS_LEN.saturating_add(entries)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxFields::new(0, FullBoxFlags::ZERO).to_bytes())?;
        let entry_count = self.entries.len() as u64;
        // Why not saturate silently: an entry count past `u32` cannot be written
        // at all, and the box has already declared a length built from it, so
        // this stands for a `Vec` no target can hold.
        writer.write_unsigned(FieldWidth::Compact, entry_count)?;

        for entry in &self.entries {
            writer.write_u32(entry.chunk_offset)?;
        }

        Ok(())
    }
}

/// One entry of the table a [`ChunkLargeOffsetBox`] holds
///
/// The offset reaches into the file that holds the media data, as a
/// [`ChunkOffsetEntry`]'s does, and is held at the width the box writes it.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ChunkLargeOffsetEntry {
    chunk_offset: u64,
}

impl ChunkLargeOffsetEntry {
    /// Creates the entry from the offset its chunk starts at
    #[must_use]
    pub const fn new(chunk_offset: u64) -> Self {
        Self { chunk_offset }
    }

    /// Returns the offset into the file the chunk starts at
    #[must_use]
    pub const fn chunk_offset(&self) -> u64 {
        self.chunk_offset
    }
}

/// Box that states where every chunk of a track lies, in 64 bits
///
/// [`ChunkLargeOffsetBox`] (`co64`), ISO/IEC 14496-12 §8.7.5. One entry per
/// chunk, in chunk order, as a [`ChunkOffsetBox`] holds them, at a width that
/// reaches any offset a file can have. The spec lets a `co64` state offsets a
/// `stco` would have reached, and such a box writes back as a `co64`.
///
/// The `entry_count` field is not held: it counts the entries, so it is derived
/// on the way out. On the way in a count that disagrees with the entries fails
/// the box.
#[doc(alias = "co64")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ChunkLargeOffsetBox {
    entries: Vec<ChunkLargeOffsetEntry>,
}

impl ChunkLargeOffsetBox {
    /// Creates the box from the entries it locates the chunks with
    #[must_use]
    pub const fn new(entries: Vec<ChunkLargeOffsetEntry>) -> Self {
        Self { entries }
    }

    /// Returns the entries, in chunk order
    #[must_use]
    pub fn entries(&self) -> &[ChunkLargeOffsetEntry] {
        &self.entries
    }
}

impl BoxDefinition for ChunkLargeOffsetBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"co64");
}

impl BoxDecode for ChunkLargeOffsetBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
    ///   ends inside a field of the box or inside one of its entries.
    /// * [`EntryCountMismatch`](isobmff_core::ErrorKind::EntryCountMismatch): the
    ///   `entry_count` field disagrees with the entries that follow it.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version != 0 {
            return Err(Error::unsupported_version(version));
        }

        let declared = u64::from(reader.read_u32()?);

        let mut entries = Vec::new();
        while !reader.remainder().is_empty() {
            entries.push(ChunkLargeOffsetEntry {
                chunk_offset: reader.read_u64()?,
            });
        }

        let actual = entries.len() as u64;
        if actual != declared {
            return Err(Error::entry_count_mismatch(declared, actual));
        }

        Ok(Self { entries })
    }
}

impl BoxEncode for ChunkLargeOffsetBox {
    fn payload_len(&self) -> u64 {
        let entries = (self.entries.len() as u64).saturating_mul(LARGE_ENTRY_LEN);

        FIXED_FIELDS_LEN.saturating_add(entries)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxFields::new(0, FullBoxFlags::ZERO).to_bytes())?;
        let entry_count = self.entries.len() as u64;
        // Why not saturate silently: an entry count past `u32` cannot be written
        // at all, and the box has already declared a length built from it, so
        // this stands for a `Vec` no target can hold.
        writer.write_unsigned(FieldWidth::Compact, entry_count)?;

        for entry in &self.entries {
            writer.write_u64(entry.chunk_offset)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, BoxVariants as _, Error, boxes};

    use super::{
        ChunkLargeOffsetBox, ChunkLargeOffsetEntry, ChunkOffsetBox, ChunkOffsetEntry, ChunkOffsets,
    };

    /// Writes the payload of a box and returns the bytes it occupies
    fn encoded_payload(chunk_offset: &impl BoxEncode) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(chunk_offset.payload_len()).unwrap()];
        chunk_offset.encode_payload(&mut buffer).unwrap();

        buffer
    }

    /// Writes the table whole, header and payload, and reads it back as the slot does
    fn round_trip(chunk_offsets: &ChunkOffsets) -> ChunkOffsets {
        let mut buffer = vec![0; usize::try_from(chunk_offsets.encoded_len()).unwrap()];
        chunk_offsets.encode(&mut buffer).unwrap();

        ChunkOffsets::decode_variant(boxes(&buffer).next().unwrap().unwrap()).unwrap()
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let chunk_offset = ChunkOffsetBox::new(vec![
            ChunkOffsetEntry::new(0x28),
            ChunkOffsetEntry::new(0x1_0000),
        ]);

        let payload = encoded_payload(&chunk_offset);

        assert_eq!(payload, b"\0\0\0\0\0\0\0\x02\0\0\0\x28\0\x01\0\0");
        assert_eq!(
            ChunkOffsetBox::decode_payload(&payload).unwrap(),
            chunk_offset
        );
    }

    #[test]
    fn a_large_offset_box_reads_back_as_the_value_that_wrote_it() {
        let chunk_large_offset = ChunkLargeOffsetBox::new(vec![
            ChunkLargeOffsetEntry::new(0x28),
            ChunkLargeOffsetEntry::new(0x1_0000_0000),
        ]);

        let payload = encoded_payload(&chunk_large_offset);

        assert_eq!(
            payload,
            b"\0\0\0\0\0\0\0\x02\0\0\0\0\0\0\0\x28\0\0\0\x01\0\0\0\0"
        );
        assert_eq!(
            ChunkLargeOffsetBox::decode_payload(&payload).unwrap(),
            chunk_large_offset
        );
    }

    #[test]
    fn a_box_holding_no_entries_declares_a_count_of_zero() {
        assert_eq!(
            encoded_payload(&ChunkOffsetBox::new(Vec::new())),
            b"\0\0\0\0\0\0\0\0"
        );
        assert_eq!(
            encoded_payload(&ChunkLargeOffsetBox::new(Vec::new())),
            b"\0\0\0\0\0\0\0\0"
        );
    }

    #[test]
    fn a_count_that_disagrees_with_the_entries_is_rejected() {
        let mut payload = encoded_payload(&ChunkOffsetBox::new(vec![ChunkOffsetEntry::new(0x28)]));
        payload
            .get_mut(4..8)
            .unwrap()
            .copy_from_slice(&4_u32.to_be_bytes());

        assert_eq!(
            ChunkOffsetBox::decode_payload(&payload),
            Err(Error::entry_count_mismatch(4, 1))
        );

        let mut payload =
            encoded_payload(&ChunkLargeOffsetBox::new(vec![ChunkLargeOffsetEntry::new(
                0x28,
            )]));
        payload
            .get_mut(4..8)
            .unwrap()
            .copy_from_slice(&4_u32.to_be_bytes());

        assert_eq!(
            ChunkLargeOffsetBox::decode_payload(&payload),
            Err(Error::entry_count_mismatch(4, 1))
        );
    }

    #[test]
    fn a_payload_ending_inside_an_entry_is_rejected() {
        assert_eq!(
            ChunkOffsetBox::decode_payload(b"\0\0\0\0\0\0\0\x01\0\0"),
            Err(Error::truncated_payload(12, 10))
        );
        assert_eq!(
            ChunkLargeOffsetBox::decode_payload(b"\0\0\0\0\0\0\0\x01\0\0\0\0"),
            Err(Error::truncated_payload(16, 12))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        assert_eq!(
            ChunkOffsetBox::decode_payload(b"\x01\0\0\0\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
        assert_eq!(
            ChunkLargeOffsetBox::decode_payload(b"\x01\0\0\0\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
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
