//! [`SyncSampleBox`] (`stss`), ISO/IEC 14496-12 §8.6.2

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWidth, FieldWriter,
    FullBoxFields, FullBoxFlags,
};

/// Length of the fields that precede the entries
const FIXED_FIELDS_LEN: u64 = 8;

/// Length of one entry of the table
const ENTRY_LEN: u64 = 4;

/// One entry of the table a [`SyncSampleBox`] holds
///
/// The entry names one sync sample of the track by its number, counted from
/// one.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SyncSampleEntry {
    sample_number: u32,
}

impl SyncSampleEntry {
    /// Creates the entry from the number of the sync sample it names
    #[must_use]
    pub const fn new(sample_number: u32) -> Self {
        Self { sample_number }
    }

    /// Returns the number of the sync sample, counted from one
    #[must_use]
    pub const fn sample_number(&self) -> u32 {
        self.sample_number
    }
}

/// Box that names the sync samples of a track
///
/// [`SyncSampleBox`] (`stss`), ISO/IEC 14496-12 §8.6.2. Every sample of a
/// track carrying no such box is a sync sample, and a box holding no entries
/// states that none is.
///
/// §8.6.2 lists the sample numbers in strictly increasing order; the box holds
/// them as it reads them and leaves the order to whoever lays them over the
/// samples.
///
/// The `entry_count` field is not held: it counts the entries, so it is derived
/// on the way out. On the way in a count that disagrees with the entries fails
/// the box.
#[doc(alias = "stss")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SyncSampleBox {
    entries: Vec<SyncSampleEntry>,
}

impl SyncSampleBox {
    /// Creates the box from the entries naming the sync samples
    #[must_use]
    pub const fn new(entries: Vec<SyncSampleEntry>) -> Self {
        Self { entries }
    }

    /// Returns the entries, in the order the box lists them
    #[must_use]
    pub fn entries(&self) -> &[SyncSampleEntry] {
        &self.entries
    }
}

impl BoxDefinition for SyncSampleBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"stss");
}

impl BoxDecode for SyncSampleBox {
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
            entries.push(SyncSampleEntry {
                sample_number: reader.read_u32()?,
            });
        }

        let actual = entries.len() as u64;
        if actual != declared {
            return Err(Error::entry_count_mismatch(declared, actual));
        }

        Ok(Self { entries })
    }
}

impl BoxEncode for SyncSampleBox {
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
            writer.write_u32(entry.sample_number)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::{SyncSampleBox, SyncSampleEntry};

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(sync_sample: &SyncSampleBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(sync_sample.payload_len()).unwrap()];
        sync_sample.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let sync_sample =
            SyncSampleBox::new(vec![SyncSampleEntry::new(1), SyncSampleEntry::new(9)]);

        let payload = encoded_payload(&sync_sample);

        assert_eq!(payload, b"\0\0\0\0\0\0\0\x02\0\0\0\x01\0\0\0\x09");
        assert_eq!(
            SyncSampleBox::decode_payload(&payload).unwrap(),
            sync_sample
        );
    }

    #[test]
    fn a_box_naming_no_sync_sample_reads_back_as_the_value_that_wrote_it() {
        let sync_sample = SyncSampleBox::new(Vec::new());

        let payload = encoded_payload(&sync_sample);

        assert_eq!(payload, b"\0\0\0\0\0\0\0\0");
        assert_eq!(
            SyncSampleBox::decode_payload(&payload).unwrap(),
            sync_sample
        );
    }

    #[test]
    fn a_count_that_disagrees_with_the_entries_is_rejected() {
        let payload = b"\0\0\0\0\0\0\0\x02\0\0\0\x01";

        assert_eq!(
            SyncSampleBox::decode_payload(payload),
            Err(Error::entry_count_mismatch(2, 1))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let payload = b"\x01\0\0\0\0\0\0\0";

        assert_eq!(
            SyncSampleBox::decode_payload(payload),
            Err(Error::unsupported_version(1))
        );
    }
}
