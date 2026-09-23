//! [`PaddingBitsBox`] (`padb`), ISO/IEC 14496-12 §8.7.6

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWidth, FieldWriter,
    FullBoxFields, FullBoxFlags,
};

/// Length of the fields that precede the entries
const FIXED_FIELDS_LEN: u64 = 8;

/// Largest number of padding bits one entry states
const PAD_MAXIMUM: u8 = 0b111;

/// One entry of the table a [`PaddingBitsBox`] holds
///
/// The entry states how many bits at the end of the sample it is indexed by
/// are padding.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PaddingBitsEntry {
    pad: u8,
}

impl PaddingBitsEntry {
    /// Creates the entry from the padding bits at the end of its sample
    ///
    /// Returns `None` when `pad` is past 7, which its 3 bits do not hold.
    #[must_use]
    pub const fn new(pad: u8) -> Option<Self> {
        if pad > PAD_MAXIMUM {
            return None;
        }

        Some(Self { pad })
    }

    /// Returns how many bits at the end of the sample are padding
    #[must_use]
    pub const fn pad(&self) -> u8 {
        self.pad
    }
}

/// Box that states how many bits at the end of each sample of a track are padding
///
/// [`PaddingBitsBox`] (`padb`), ISO/IEC 14496-12 §8.7.6. The table holds one
/// entry per sample, which the wire packs two to a byte; the second half of
/// the last byte of a table counting an odd number of samples is read as
/// nothing and written as zero.
///
/// The `sample_count` field is not held: it counts the entries, so it is
/// derived on the way out. On the way in a count that disagrees with the
/// bytes that follow it fails the box.
#[doc(alias = "padb")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct PaddingBitsBox {
    entries: Vec<PaddingBitsEntry>,
}

impl PaddingBitsBox {
    /// Creates the box from the entry of every sample in turn
    #[must_use]
    pub const fn new(entries: Vec<PaddingBitsEntry>) -> Self {
        Self { entries }
    }

    /// Returns the entries, one per sample in decode order
    #[must_use]
    pub fn entries(&self) -> &[PaddingBitsEntry] {
        &self.entries
    }
}

impl BoxDefinition for PaddingBitsBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"padb");
}

impl BoxDecode for PaddingBitsBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
    ///   ends inside a field of the box.
    /// * [`EntryCountMismatch`](isobmff_core::ErrorKind::EntryCountMismatch): the
    ///   `sample_count` field disagrees with the bytes that follow it, which
    ///   hold two samples each.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version != 0 {
            return Err(Error::unsupported_version(version));
        }

        let declared = u64::from(reader.read_u32()?);
        let packed = reader.take_remainder();
        if declared.div_ceil(2) != packed.len() as u64 {
            return Err(Error::entry_count_mismatch(
                declared,
                (packed.len() as u64).saturating_mul(2),
            ));
        }

        let mut entries: Vec<PaddingBitsEntry> = packed
            .iter()
            .flat_map(|byte| [byte >> 4, *byte])
            .map(|pad| PaddingBitsEntry {
                pad: pad & PAD_MAXIMUM,
            })
            .collect();
        if declared % 2 == 1 {
            entries.pop();
        }

        Ok(Self { entries })
    }
}

impl BoxEncode for PaddingBitsBox {
    fn payload_len(&self) -> u64 {
        FIXED_FIELDS_LEN.saturating_add((self.entries.len() as u64).div_ceil(2))
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxFields::new(0, FullBoxFlags::ZERO).to_bytes())?;
        let sample_count = self.entries.len() as u64;
        // Why not saturate silently: a sample count past `u32` cannot be written
        // at all, and the box has already declared a length built from it, so
        // this stands for a `Vec` no target can hold.
        writer.write_unsigned(FieldWidth::Compact, sample_count)?;

        for pair in self.entries.chunks(2) {
            let pad1 = pair.first().map_or(0, PaddingBitsEntry::pad);
            let pad2 = pair.get(1).map_or(0, PaddingBitsEntry::pad);
            writer.write_bytes(&[pad1 << 4 | pad2])?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::{PaddingBitsBox, PaddingBitsEntry};

    /// Entry stating `pad` bits of padding, which its 3 bits hold
    fn padded(pad: u8) -> PaddingBitsEntry {
        PaddingBitsEntry::new(pad).unwrap()
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(padding_bits: &PaddingBitsBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(padding_bits.payload_len()).unwrap()];
        padding_bits.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let padding_bits = PaddingBitsBox::new(vec![padded(7), padded(1), padded(3), padded(0)]);

        let payload = encoded_payload(&padding_bits);

        assert_eq!(payload, b"\0\0\0\0\0\0\0\x04\x71\x30");
        assert_eq!(
            PaddingBitsBox::decode_payload(&payload).unwrap(),
            padding_bits
        );
    }

    #[test]
    fn a_box_counting_an_odd_number_of_samples_writes_the_last_half_byte_as_zero() {
        let padding_bits = PaddingBitsBox::new(vec![padded(7), padded(1), padded(3)]);

        let payload = encoded_payload(&padding_bits);

        assert_eq!(payload, b"\0\0\0\0\0\0\0\x03\x71\x30");
        assert_eq!(
            PaddingBitsBox::decode_payload(&payload).unwrap(),
            padding_bits
        );
    }

    #[test]
    fn a_box_holding_no_entries_reads_back_as_the_value_that_wrote_it() {
        let padding_bits = PaddingBitsBox::new(Vec::new());

        let payload = encoded_payload(&padding_bits);

        assert_eq!(payload, b"\0\0\0\0\0\0\0\0");
        assert_eq!(
            PaddingBitsBox::decode_payload(&payload).unwrap(),
            padding_bits
        );
    }

    #[test]
    fn a_count_that_disagrees_with_the_bytes_that_follow_is_rejected() {
        let payload = b"\0\0\0\0\0\0\0\x03\x71";

        assert_eq!(
            PaddingBitsBox::decode_payload(payload),
            Err(Error::entry_count_mismatch(3, 2))
        );
    }

    #[test]
    fn padding_past_its_three_bits_builds_no_entry() {
        assert_eq!(PaddingBitsEntry::new(8), None);
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let payload = b"\x01\0\0\0\0\0\0\0";

        assert_eq!(
            PaddingBitsBox::decode_payload(payload),
            Err(Error::unsupported_version(1))
        );
    }
}
