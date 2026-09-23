//! [`DegradationPriorityBox`] (`stdp`), ISO/IEC 14496-12 §8.5.3

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFields,
    FullBoxFlags,
};

/// Length of the fields that precede the entries
const FIXED_FIELDS_LEN: u64 = 4;

/// Length of one entry of the table
const ENTRY_LEN: u64 = 2;

/// One entry of the table a [`DegradationPriorityBox`] holds
///
/// The entry states the degradation priority of the sample it is indexed by,
/// whose meaning and range the specifications derived from ISO/IEC 14496-12
/// define.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DegradationPriorityEntry {
    priority: u16,
}

impl DegradationPriorityEntry {
    /// Creates the entry from the degradation priority of its sample
    #[must_use]
    pub const fn new(priority: u16) -> Self {
        Self { priority }
    }

    /// Returns the degradation priority of the sample
    #[must_use]
    pub const fn priority(&self) -> u16 {
        self.priority
    }
}

/// Box that states the degradation priority of each sample of a track
///
/// [`DegradationPriorityBox`] (`stdp`), ISO/IEC 14496-12 §8.5.3. The table
/// holds one entry per sample and states no count: §8.5.3 takes it from the
/// sample size box, so the box holds as many entries as its payload has room
/// for.
#[doc(alias = "stdp")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct DegradationPriorityBox {
    entries: Vec<DegradationPriorityEntry>,
}

impl DegradationPriorityBox {
    /// Creates the box from the entry of every sample in turn
    #[must_use]
    pub const fn new(entries: Vec<DegradationPriorityEntry>) -> Self {
        Self { entries }
    }

    /// Returns the entries, one per sample in decode order
    #[must_use]
    pub fn entries(&self) -> &[DegradationPriorityEntry] {
        &self.entries
    }
}

impl BoxDefinition for DegradationPriorityBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"stdp");
}

impl BoxDecode for DegradationPriorityBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
    ///   ends inside the version and flags or inside one of the entries.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version != 0 {
            return Err(Error::unsupported_version(version));
        }

        let mut entries = Vec::new();
        while !reader.remainder().is_empty() {
            entries.push(DegradationPriorityEntry {
                priority: reader.read_u16()?,
            });
        }

        Ok(Self { entries })
    }
}

impl BoxEncode for DegradationPriorityBox {
    fn payload_len(&self) -> u64 {
        let entries = (self.entries.len() as u64).saturating_mul(ENTRY_LEN);

        FIXED_FIELDS_LEN.saturating_add(entries)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxFields::new(0, FullBoxFlags::ZERO).to_bytes())?;

        for entry in &self.entries {
            writer.write_u16(entry.priority)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::{DegradationPriorityBox, DegradationPriorityEntry};

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(degradation_priority: &DegradationPriorityBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(degradation_priority.payload_len()).unwrap()];
        degradation_priority.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let degradation_priority = DegradationPriorityBox::new(vec![
            DegradationPriorityEntry::new(0x0102),
            DegradationPriorityEntry::new(0xffff),
        ]);

        let payload = encoded_payload(&degradation_priority);

        assert_eq!(payload, b"\0\0\0\0\x01\x02\xff\xff");
        assert_eq!(
            DegradationPriorityBox::decode_payload(&payload).unwrap(),
            degradation_priority
        );
    }

    #[test]
    fn a_box_holding_no_entries_reads_back_as_the_value_that_wrote_it() {
        let degradation_priority = DegradationPriorityBox::new(Vec::new());

        let payload = encoded_payload(&degradation_priority);

        assert_eq!(payload, b"\0\0\0\0");
        assert_eq!(
            DegradationPriorityBox::decode_payload(&payload).unwrap(),
            degradation_priority
        );
    }

    #[test]
    fn a_payload_ending_inside_an_entry_is_rejected() {
        let payload = b"\0\0\0\0\x01\x02\xff";

        assert_eq!(
            DegradationPriorityBox::decode_payload(payload),
            Err(Error::truncated_payload(8, 7))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let payload = b"\x01\0\0\0";

        assert_eq!(
            DegradationPriorityBox::decode_payload(payload),
            Err(Error::unsupported_version(1))
        );
    }
}
