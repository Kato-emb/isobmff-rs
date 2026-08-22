//! [`ChunkOffsetBox`] (`stco`), ISO/IEC 14496-12 §8.7.5

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWidth, FieldWriter,
    FullBoxFields, FullBoxFlags,
};

/// Length of the fields that precede the entries
const FIXED_FIELDS_LEN: u64 = 8;

/// Length of one entry of the table
const ENTRY_LEN: u64 = 4;

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

/// Box that states where every chunk of a track lies
///
/// [`ChunkOffsetBox`] (`stco`), ISO/IEC 14496-12 §8.7.5. One entry per chunk,
/// in chunk order, which the `stsc` maps the samples onto. The offsets are
/// 32-bit; a presentation whose chunks start past 4 GiB states them in a `co64`
/// instead, which is a box of its own.
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

        let actual = u64::try_from(entries.len()).unwrap_or(u64::MAX);
        if actual != declared {
            return Err(Error::entry_count_mismatch(declared, actual));
        }

        Ok(Self { entries })
    }
}

impl BoxEncode for ChunkOffsetBox {
    fn payload_len(&self) -> u64 {
        let entries = u64::try_from(self.entries.len())
            .unwrap_or(u64::MAX)
            .saturating_mul(ENTRY_LEN);

        FIXED_FIELDS_LEN.saturating_add(entries)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxFields::new(0, FullBoxFlags::ZERO).to_bytes())?;
        let entry_count = u64::try_from(self.entries.len()).unwrap_or(u64::MAX);
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

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::{ChunkOffsetBox, ChunkOffsetEntry};

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(chunk_offset: &ChunkOffsetBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(chunk_offset.payload_len()).unwrap()];
        chunk_offset.encode_payload(&mut buffer).unwrap();

        buffer
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
    fn a_box_holding_no_entries_declares_a_count_of_zero() {
        let payload = encoded_payload(&ChunkOffsetBox::new(Vec::new()));

        assert_eq!(payload, b"\0\0\0\0\0\0\0\0");
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
    }

    #[test]
    fn a_payload_ending_inside_an_entry_is_rejected() {
        let payload = b"\0\0\0\0\0\0\0\x01\0\0";

        assert_eq!(
            ChunkOffsetBox::decode_payload(payload),
            Err(Error::truncated_payload(12, 10))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let payload = b"\x01\0\0\0\0\0\0\0";

        assert_eq!(
            ChunkOffsetBox::decode_payload(payload),
            Err(Error::unsupported_version(1))
        );
    }
}
