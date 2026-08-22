//! [`SampleToChunkBox`] (`stsc`), ISO/IEC 14496-12 §8.7.4

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWidth, FieldWriter,
    FullBoxFields, FullBoxFlags,
};

/// Length of the fields that precede the entries
const FIXED_FIELDS_LEN: u64 = 8;

/// Length of one entry of the table
const ENTRY_LEN: u64 = 12;

/// One entry of the table a [`SampleToChunkBox`] holds
///
/// The entry opens a run of chunks that hold the same number of samples and
/// share one sample description: `first_chunk` is the chunk the run starts at,
/// and the run reaches the chunk the next entry starts at. Chunks are numbered
/// from one.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SampleToChunkEntry {
    first_chunk: u32,
    samples_per_chunk: u32,
    sample_description_index: u32,
}

impl SampleToChunkEntry {
    /// Creates the entry from the chunk it starts at and what that run holds
    #[must_use]
    pub const fn new(
        first_chunk: u32,
        samples_per_chunk: u32,
        sample_description_index: u32,
    ) -> Self {
        Self {
            first_chunk,
            samples_per_chunk,
            sample_description_index,
        }
    }

    /// Returns the chunk this run of chunks starts at
    #[must_use]
    pub const fn first_chunk(&self) -> u32 {
        self.first_chunk
    }

    /// Returns how many samples each chunk of the run holds
    #[must_use]
    pub const fn samples_per_chunk(&self) -> u32 {
        self.samples_per_chunk
    }

    /// Returns the `stsd` entry the samples of the run are described by
    #[must_use]
    pub const fn sample_description_index(&self) -> u32 {
        self.sample_description_index
    }
}

/// Box that states which chunk of a track each of its samples lies in
///
/// [`SampleToChunkBox`] (`stsc`), ISO/IEC 14496-12 §8.7.4. Samples are grouped
/// into chunks, and the table run-length codes that grouping: each
/// [`SampleToChunkEntry`] opens a run of chunks holding the same number of
/// samples, so finding the chunk of a sample is walking the runs.
///
/// The table is held as the file states it. §8.7.4 has the first entry start at
/// chunk one and the entries run in increasing chunk order, and a table that
/// does not is carried through rather than refused.
///
/// The `entry_count` field is not held: it counts the entries, so it is derived
/// on the way out. On the way in a count that disagrees with the entries fails
/// the box.
#[doc(alias = "stsc")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SampleToChunkBox {
    entries: Vec<SampleToChunkEntry>,
}

impl SampleToChunkBox {
    /// Creates the box from the entries it groups the samples with
    #[must_use]
    pub const fn new(entries: Vec<SampleToChunkEntry>) -> Self {
        Self { entries }
    }

    /// Returns the entries, in the order the runs of chunks follow one another
    #[must_use]
    pub fn entries(&self) -> &[SampleToChunkEntry] {
        &self.entries
    }
}

impl BoxDefinition for SampleToChunkBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"stsc");
}

impl BoxDecode for SampleToChunkBox {
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
            entries.push(SampleToChunkEntry {
                first_chunk: reader.read_u32()?,
                samples_per_chunk: reader.read_u32()?,
                sample_description_index: reader.read_u32()?,
            });
        }

        let actual = entries.len() as u64;
        if actual != declared {
            return Err(Error::entry_count_mismatch(declared, actual));
        }

        Ok(Self { entries })
    }
}

impl BoxEncode for SampleToChunkBox {
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
            writer.write_u32(entry.first_chunk)?;
            writer.write_u32(entry.samples_per_chunk)?;
            writer.write_u32(entry.sample_description_index)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::{SampleToChunkBox, SampleToChunkEntry};

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(sample_to_chunk: &SampleToChunkBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(sample_to_chunk.payload_len()).unwrap()];
        sample_to_chunk.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let sample_to_chunk = SampleToChunkBox::new(vec![
            SampleToChunkEntry::new(1, 4, 1),
            SampleToChunkEntry::new(3, 2, 1),
        ]);

        let payload = encoded_payload(&sample_to_chunk);

        assert_eq!(
            payload,
            b"\0\0\0\0\0\0\0\x02\0\0\0\x01\0\0\0\x04\0\0\0\x01\0\0\0\x03\0\0\0\x02\0\0\0\x01"
        );
        assert_eq!(
            SampleToChunkBox::decode_payload(&payload).unwrap(),
            sample_to_chunk
        );
    }

    #[test]
    fn a_box_holding_no_entries_declares_a_count_of_zero() {
        let payload = encoded_payload(&SampleToChunkBox::new(Vec::new()));

        assert_eq!(payload, b"\0\0\0\0\0\0\0\0");
    }

    #[test]
    fn a_table_the_spec_orders_otherwise_is_carried_through() {
        let sample_to_chunk = SampleToChunkBox::new(vec![
            SampleToChunkEntry::new(7, 1, 1),
            SampleToChunkEntry::new(2, 1, 1),
        ]);

        let payload = encoded_payload(&sample_to_chunk);

        assert_eq!(
            SampleToChunkBox::decode_payload(&payload).unwrap(),
            sample_to_chunk
        );
    }

    #[test]
    fn a_count_that_disagrees_with_the_entries_is_rejected() {
        let mut payload = encoded_payload(&SampleToChunkBox::new(vec![SampleToChunkEntry::new(
            1, 4, 1,
        )]));
        payload
            .get_mut(4..8)
            .unwrap()
            .copy_from_slice(&4_u32.to_be_bytes());

        assert_eq!(
            SampleToChunkBox::decode_payload(&payload),
            Err(Error::entry_count_mismatch(4, 1))
        );
    }

    #[test]
    fn a_payload_ending_inside_an_entry_is_rejected() {
        let payload = b"\0\0\0\0\0\0\0\x01\0\0\0\x01\0\0\0\x04";

        assert_eq!(
            SampleToChunkBox::decode_payload(payload),
            Err(Error::truncated_payload(20, 16))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let payload = b"\x01\0\0\0\0\0\0\0";

        assert_eq!(
            SampleToChunkBox::decode_payload(payload),
            Err(Error::unsupported_version(1))
        );
    }
}
