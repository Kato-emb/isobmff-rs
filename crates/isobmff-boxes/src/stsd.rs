//! [`SampleDescriptionBox`] (`stsd`), ISO/IEC 14496-12 §8.5.2

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWidth,
    FieldWriter, FullBoxFields, FullBoxFlags, boxes,
};

/// Length of the fields that precede the entries
const FIXED_FIELDS_LEN: u64 = 8;

/// Box that describes the coding every sample of a track was made with
///
/// [`SampleDescriptionBox`] (`stsd`), ISO/IEC 14496-12 §8.5.2. Each entry is a
/// box whose type names a coding, and whose payload the coding's own
/// specification lays out — so the entries are kept as [`AnyBox`] and left
/// unread. A reader that knows a coding decodes an entry itself.
///
/// The `entry_count` field is not held: it counts the entries, so it is derived
/// on the way out. On the way in a count that disagrees with the entries fails
/// the box.
#[doc(alias = "stsd")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct SampleDescriptionBox {
    entries: Vec<AnyBox>,
}

impl SampleDescriptionBox {
    /// Creates the box from the entries it describes the samples with
    #[must_use]
    pub const fn new(entries: Vec<AnyBox>) -> Self {
        Self { entries }
    }

    /// Returns the entries, each naming a coding and carrying its own fields
    #[must_use]
    pub fn entries(&self) -> &[AnyBox] {
        &self.entries
    }
}

impl BoxDefinition for SampleDescriptionBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"stsd");
}

impl BoxDecode for SampleDescriptionBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends before the fields that precede the entries.
    /// * The failures of [`boxes`]: an entry does not frame as a box.
    /// * [`EntryCountMismatch`](isobmff_core::ErrorKind::EntryCountMismatch): the
    ///   `entry_count` field disagrees with the entries that follow it.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version != 0 {
            return Err(Error::unsupported_version(version));
        }

        let declared = u64::from(reader.read_u32()?);

        let mut entries = Vec::new();
        for entry in boxes(reader.take_remainder()) {
            let entry = entry?;
            entries.push(AnyBox::from_raw_bytes(
                entry.header().box_type(),
                entry.payload().to_vec(),
            ));
        }

        let actual = u64::try_from(entries.len()).unwrap_or(u64::MAX);
        if actual != declared {
            return Err(Error::entry_count_mismatch(declared, actual));
        }

        Ok(Self { entries })
    }
}

impl BoxEncode for SampleDescriptionBox {
    fn payload_len(&self) -> u64 {
        let entries = self.entries.iter().fold(0_u64, |total, entry| {
            total.saturating_add(entry.encoded_len())
        });

        FIXED_FIELDS_LEN.saturating_add(entries)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxFields::new(0, FullBoxFlags::ZERO).to_bytes())?;
        let entry_count = u64::try_from(self.entries.len()).unwrap_or(u64::MAX);
        // Why not saturate silently: an entry count past `u32` cannot be written
        // at all, and the box has already declared a length built from it, so
        // this stands for a `Vec` no target can hold.
        writer.write_unsigned(FieldWidth::Compact, entry_count)?;

        let mut rest = writer.take_remainder();
        for entry in &self.entries {
            rest = entry.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{AnyBox, BoxDecode, BoxEncode, BoxType, Error};

    use super::SampleDescriptionBox;

    /// Sample entry for a coding this crate has no type for
    fn avc_entry() -> AnyBox {
        AnyBox::from_raw_bytes(BoxType::compact(*b"avc1"), vec![0xab; 4])
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(description: &SampleDescriptionBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(description.payload_len()).unwrap()];
        description.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let description = SampleDescriptionBox::new(vec![avc_entry()]);

        let payload = encoded_payload(&description);

        assert_eq!(payload, b"\0\0\0\0\0\0\0\x01\0\0\0\x0cavc1\xab\xab\xab\xab");
        assert_eq!(
            SampleDescriptionBox::decode_payload(&payload).unwrap(),
            description
        );
    }

    #[test]
    fn the_entry_count_is_written_from_the_entries_the_box_holds() {
        let description = SampleDescriptionBox::new(vec![avc_entry(), avc_entry()]);

        let payload = encoded_payload(&description);

        assert_eq!(payload.get(4..8), Some(b"\0\0\0\x02".as_slice()));
    }

    #[test]
    fn a_box_holding_no_entries_declares_a_count_of_zero() {
        let payload = encoded_payload(&SampleDescriptionBox::new(Vec::new()));

        assert_eq!(payload, b"\0\0\0\0\0\0\0\0");
    }

    #[test]
    fn a_count_that_disagrees_with_the_entries_is_rejected() {
        let mut payload = encoded_payload(&SampleDescriptionBox::new(vec![avc_entry()]));
        payload
            .get_mut(4..8)
            .unwrap()
            .copy_from_slice(&4_u32.to_be_bytes());

        assert_eq!(
            SampleDescriptionBox::decode_payload(&payload),
            Err(Error::entry_count_mismatch(4, 1))
        );
    }

    #[test]
    fn an_entry_that_does_not_frame_as_a_box_is_rejected() {
        let payload = b"\0\0\0\0\0\0\0\x01\0\0\0\x20avc1";

        assert_eq!(
            SampleDescriptionBox::decode_payload(payload),
            Err(Error::truncated_box(32, 8))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        assert_eq!(
            SampleDescriptionBox::decode_payload(b"\x01\0\0\0\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
