//! [`DataReferenceBox`] (`dref`), ISO/IEC 14496-12 §8.7.2

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWidth, FieldWriter,
    FullBoxFields, FullBoxFlags, boxes,
};

use crate::data_entry::DataEntry;

/// Length of the fields that precede the entries
const FIXED_FIELDS_LEN: u64 = 8;

/// Box that tables where the media data a track references lies
///
/// [`DataReferenceBox`] (`dref`), ISO/IEC 14496-12 §8.7.2. Each entry declares
/// one location, and the `sample_description_index` of a sample ties it to the
/// entry its data lies at, so one track may draw on several sources.
///
/// The `entry_count` field is not held: it counts the entries, so it is derived
/// on the way out. On the way in a count that disagrees with the entries fails
/// the box. §8.7.2.1 has the count be 1 or greater, and decoding does **not**
/// enforce it.
///
/// Neither the version nor the `flags` are held — the spec declares both zero
/// for this box. The version and the flags of an *entry* belong to the entry.
#[doc(alias = "dref")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct DataReferenceBox {
    entries: Vec<DataEntry>,
}

impl DataReferenceBox {
    /// Creates the box from the entries it locates the media data with
    #[must_use]
    pub const fn new(entries: Vec<DataEntry>) -> Self {
        Self { entries }
    }

    /// Returns the entries, each declaring one place the media data lies
    #[must_use]
    pub fn entries(&self) -> &[DataEntry] {
        &self.entries
    }
}

impl BoxDefinition for DataReferenceBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"dref");
}

impl BoxDecode for DataReferenceBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends before the fields that precede the entries.
    /// * The failures of [`boxes`]: an entry does not frame as a box.
    /// * [`ForbiddenChildBox`](isobmff_core::ErrorKind::ForbiddenChildBox): an entry
    ///   is neither a `url_` nor a `urn_`, which §8.7.2.1 closes the set to.
    /// * [`EntryCountMismatch`](isobmff_core::ErrorKind::EntryCountMismatch): the
    ///   `entry_count` field disagrees with the entries that follow it.
    /// * Whatever an entry reports, on the [`containers`](Error::containers) path.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version != 0 {
            return Err(Error::unsupported_version(version));
        }

        let declared = u64::from(reader.read_u32()?);

        let mut entries = Vec::new();
        for entry in boxes(reader.take_remainder()) {
            entries.push(DataEntry::decode(entry?)?);
        }

        let actual = entries.len() as u64;
        if actual != declared {
            return Err(Error::entry_count_mismatch(declared, actual));
        }

        Ok(Self { entries })
    }
}

impl BoxEncode for DataReferenceBox {
    fn payload_len(&self) -> u64 {
        let entries = self.entries.iter().fold(0_u64, |total, entry| {
            total.saturating_add(entry.encoded_len())
        });

        FIXED_FIELDS_LEN.saturating_add(entries)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxFields::new(0, FullBoxFlags::ZERO).to_bytes())?;
        let entry_count = self.entries.len() as u64;
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
pub(crate) mod tests {
    use alloc::string::String;
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, BoxType, Error, NullTerminatedString};

    use super::DataReferenceBox;
    use crate::data_entry::{DataEntry, DataEntryUrlBox};

    /// Data reference of a track whose data lies in the file it is read from
    pub(crate) fn data_reference() -> DataReferenceBox {
        DataReferenceBox::new(vec![DataEntry::Url(DataEntryUrlBox::new(None))])
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(data_reference: &DataReferenceBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(data_reference.payload_len()).unwrap()];
        data_reference.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let payload = encoded_payload(&data_reference());

        assert_eq!(payload, b"\0\0\0\0\0\0\0\x01\0\0\0\x0curl \0\0\0\x01");
        assert_eq!(
            DataReferenceBox::decode_payload(&payload).unwrap(),
            data_reference()
        );
    }

    #[test]
    fn the_entry_count_is_written_from_the_entries_the_box_holds() {
        let elsewhere = DataEntryUrlBox::new(Some(
            NullTerminatedString::new(String::from("media.mp4")).unwrap(),
        ));
        let data_reference = DataReferenceBox::new(vec![
            DataEntry::Url(elsewhere),
            DataEntry::Url(DataEntryUrlBox::new(None)),
        ]);

        let payload = encoded_payload(&data_reference);

        assert_eq!(payload.get(4..8), Some(b"\0\0\0\x02".as_slice()));
        assert_eq!(
            DataReferenceBox::decode_payload(&payload).unwrap(),
            data_reference
        );
    }

    #[test]
    fn a_count_that_disagrees_with_the_entries_is_rejected() {
        let mut payload = encoded_payload(&data_reference());
        payload
            .get_mut(4..8)
            .unwrap()
            .copy_from_slice(&4_u32.to_be_bytes());

        assert_eq!(
            DataReferenceBox::decode_payload(&payload),
            Err(Error::entry_count_mismatch(4, 1))
        );
    }

    #[test]
    fn an_entry_of_a_type_the_spec_does_not_allow_here_is_rejected() {
        let payload = b"\0\0\0\0\0\0\0\x01\0\0\0\x0cfree\0\0\0\0";

        assert_eq!(
            DataReferenceBox::decode_payload(payload),
            Err(Error::forbidden_child_box(BoxType::compact(*b"free")))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        assert_eq!(
            DataReferenceBox::decode_payload(b"\x01\0\0\0\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
