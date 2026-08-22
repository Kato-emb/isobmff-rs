//! [`TimeToSampleBox`] (`stts`), ISO/IEC 14496-12 §8.6.1.2

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWidth, FieldWriter,
    FullBoxFields, FullBoxFlags,
};

/// Length of the fields that precede the entries
const FIXED_FIELDS_LEN: u64 = 8;

/// Length of one entry of the table
const ENTRY_LEN: u64 = 8;

/// One entry of the table a [`TimeToSampleBox`] holds
///
/// The entry counts the samples that follow one another with the same decode
/// time delta and states that delta, so the table run-length codes a delta per
/// sample.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TimeToSampleEntry {
    sample_count: u32,
    sample_delta: u32,
}

impl TimeToSampleEntry {
    /// Creates the entry from the samples it counts and the delta they share
    #[must_use]
    pub const fn new(sample_count: u32, sample_delta: u32) -> Self {
        Self {
            sample_count,
            sample_delta,
        }
    }

    /// Returns how many samples in a row carry this delta
    #[must_use]
    pub const fn sample_count(&self) -> u32 {
        self.sample_count
    }

    /// Returns the delta those samples carry, in the media time scale
    #[must_use]
    pub const fn sample_delta(&self) -> u32 {
        self.sample_delta
    }
}

/// Box that states the decode time of every sample of a track as a delta
///
/// [`TimeToSampleBox`] (`stts`), ISO/IEC 14496-12 §8.6.1.2. The decode time of
/// a sample is the sum of the deltas before it, so the entries build the whole
/// decode timeline of the track and the sum of them all is the length of its
/// media. The deltas are ordered by decode time, which is what makes them
/// non-negative.
///
/// The `entry_count` field is not held: it counts the entries, so it is derived
/// on the way out. On the way in a count that disagrees with the entries fails
/// the box.
#[doc(alias = "stts")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TimeToSampleBox {
    entries: Vec<TimeToSampleEntry>,
}

impl TimeToSampleBox {
    /// Creates the box from the entries it states the decode timeline with
    #[must_use]
    pub const fn new(entries: Vec<TimeToSampleEntry>) -> Self {
        Self { entries }
    }

    /// Returns the entries, in the order the decode timeline runs
    #[must_use]
    pub fn entries(&self) -> &[TimeToSampleEntry] {
        &self.entries
    }
}

impl BoxDefinition for TimeToSampleBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"stts");
}

impl BoxDecode for TimeToSampleBox {
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
            entries.push(TimeToSampleEntry {
                sample_count: reader.read_u32()?,
                sample_delta: reader.read_u32()?,
            });
        }

        let actual = u64::try_from(entries.len()).unwrap_or(u64::MAX);
        if actual != declared {
            return Err(Error::entry_count_mismatch(declared, actual));
        }

        Ok(Self { entries })
    }
}

impl BoxEncode for TimeToSampleBox {
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
            writer.write_u32(entry.sample_count)?;
            writer.write_u32(entry.sample_delta)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::{TimeToSampleBox, TimeToSampleEntry};

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(time_to_sample: &TimeToSampleBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(time_to_sample.payload_len()).unwrap()];
        time_to_sample.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let time_to_sample = TimeToSampleBox::new(vec![
            TimeToSampleEntry::new(14, 10),
            TimeToSampleEntry::new(1, 5),
        ]);

        let payload = encoded_payload(&time_to_sample);

        assert_eq!(
            payload,
            b"\0\0\0\0\0\0\0\x02\0\0\0\x0e\0\0\0\x0a\0\0\0\x01\0\0\0\x05"
        );
        assert_eq!(
            TimeToSampleBox::decode_payload(&payload).unwrap(),
            time_to_sample
        );
    }

    #[test]
    fn a_box_holding_no_entries_declares_a_count_of_zero() {
        let payload = encoded_payload(&TimeToSampleBox::new(Vec::new()));

        assert_eq!(payload, b"\0\0\0\0\0\0\0\0");
    }

    #[test]
    fn a_count_that_disagrees_with_the_entries_is_rejected() {
        let mut payload =
            encoded_payload(&TimeToSampleBox::new(vec![TimeToSampleEntry::new(14, 10)]));
        payload
            .get_mut(4..8)
            .unwrap()
            .copy_from_slice(&4_u32.to_be_bytes());

        assert_eq!(
            TimeToSampleBox::decode_payload(&payload),
            Err(Error::entry_count_mismatch(4, 1))
        );
    }

    #[test]
    fn a_payload_ending_inside_an_entry_is_rejected() {
        let payload = b"\0\0\0\0\0\0\0\x01\0\0\0\x0e";

        assert_eq!(
            TimeToSampleBox::decode_payload(payload),
            Err(Error::truncated_payload(16, 12))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let payload = b"\x01\0\0\0\0\0\0\0";

        assert_eq!(
            TimeToSampleBox::decode_payload(payload),
            Err(Error::unsupported_version(1))
        );
    }
}
