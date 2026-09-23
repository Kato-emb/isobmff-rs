//! [`SampleDependencyTypeBox`] (`sdtp`), ISO/IEC 14496-12 §8.6.4

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFields,
    FullBoxFlags,
};

/// Length of the fields that precede the entries
const FIXED_FIELDS_LEN: u64 = 4;

/// Largest value one of the 2-bit fields of an entry holds
const FIELD_MAXIMUM: u8 = 0b11;

/// One entry of the table a [`SampleDependencyTypeBox`] holds
///
/// The entry states how the sample it is indexed by stands to the samples
/// around it, each field in the 2 bits §8.6.4 gives it, where 0 leaves the
/// answer unknown.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SampleDependencyTypeEntry {
    is_leading: u8,
    sample_depends_on: u8,
    sample_is_depended_on: u8,
    sample_has_redundancy: u8,
}

impl SampleDependencyTypeEntry {
    /// Creates the entry from the four answers it states for its sample
    ///
    /// Returns `None` when one of them is past 3, which its 2 bits do not
    /// hold.
    #[must_use]
    pub const fn new(
        is_leading: u8,
        sample_depends_on: u8,
        sample_is_depended_on: u8,
        sample_has_redundancy: u8,
    ) -> Option<Self> {
        if is_leading > FIELD_MAXIMUM
            || sample_depends_on > FIELD_MAXIMUM
            || sample_is_depended_on > FIELD_MAXIMUM
            || sample_has_redundancy > FIELD_MAXIMUM
        {
            return None;
        }

        Some(Self {
            is_leading,
            sample_depends_on,
            sample_is_depended_on,
            sample_has_redundancy,
        })
    }

    /// Returns whether the sample is a leading sample, and whether it decodes as one
    #[must_use]
    pub const fn is_leading(&self) -> u8 {
        self.is_leading
    }

    /// Returns whether the sample depends on others
    #[must_use]
    pub const fn sample_depends_on(&self) -> u8 {
        self.sample_depends_on
    }

    /// Returns whether other samples depend on this one
    #[must_use]
    pub const fn sample_is_depended_on(&self) -> u8 {
        self.sample_is_depended_on
    }

    /// Returns whether the sample carries redundant codings
    #[must_use]
    pub const fn sample_has_redundancy(&self) -> u8 {
        self.sample_has_redundancy
    }
}

/// Box that states how each sample of a track depends on the others
///
/// [`SampleDependencyTypeBox`] (`sdtp`), ISO/IEC 14496-12 §8.6.4. The table
/// holds one entry per sample and states no count: §8.6.4 takes it from the
/// sample size box, so the box holds as many entries as its payload has
/// bytes.
#[doc(alias = "sdtp")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SampleDependencyTypeBox {
    entries: Vec<SampleDependencyTypeEntry>,
}

impl SampleDependencyTypeBox {
    /// Creates the box from the entry of every sample in turn
    #[must_use]
    pub const fn new(entries: Vec<SampleDependencyTypeEntry>) -> Self {
        Self { entries }
    }

    /// Returns the entries, one per sample in decode order
    #[must_use]
    pub fn entries(&self) -> &[SampleDependencyTypeEntry] {
        &self.entries
    }
}

impl BoxDefinition for SampleDependencyTypeBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"sdtp");
}

impl BoxDecode for SampleDependencyTypeBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
    ///   ends inside the version and flags.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version != 0 {
            return Err(Error::unsupported_version(version));
        }

        let entries = reader
            .take_remainder()
            .iter()
            .map(|byte| SampleDependencyTypeEntry {
                is_leading: byte >> 6,
                sample_depends_on: (byte >> 4) & FIELD_MAXIMUM,
                sample_is_depended_on: (byte >> 2) & FIELD_MAXIMUM,
                sample_has_redundancy: byte & FIELD_MAXIMUM,
            })
            .collect();

        Ok(Self { entries })
    }
}

impl BoxEncode for SampleDependencyTypeBox {
    fn payload_len(&self) -> u64 {
        FIXED_FIELDS_LEN.saturating_add(self.entries.len() as u64)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxFields::new(0, FullBoxFlags::ZERO).to_bytes())?;

        for entry in &self.entries {
            writer.write_bytes(&[entry.is_leading << 6
                | entry.sample_depends_on << 4
                | entry.sample_is_depended_on << 2
                | entry.sample_has_redundancy])?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::{SampleDependencyTypeBox, SampleDependencyTypeEntry};

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(sample_dependency_type: &SampleDependencyTypeBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(sample_dependency_type.payload_len()).unwrap()];
        sample_dependency_type.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let sample_dependency_type = SampleDependencyTypeBox::new(vec![
            SampleDependencyTypeEntry::new(2, 2, 1, 0).unwrap(),
            SampleDependencyTypeEntry::new(1, 1, 2, 3).unwrap(),
        ]);

        let payload = encoded_payload(&sample_dependency_type);

        assert_eq!(payload, b"\0\0\0\0\xa4\x5b");
        assert_eq!(
            SampleDependencyTypeBox::decode_payload(&payload).unwrap(),
            sample_dependency_type
        );
    }

    #[test]
    fn a_box_holding_no_entries_reads_back_as_the_value_that_wrote_it() {
        let sample_dependency_type = SampleDependencyTypeBox::new(Vec::new());

        let payload = encoded_payload(&sample_dependency_type);

        assert_eq!(payload, b"\0\0\0\0");
        assert_eq!(
            SampleDependencyTypeBox::decode_payload(&payload).unwrap(),
            sample_dependency_type
        );
    }

    #[test]
    fn an_answer_past_its_two_bits_builds_no_entry() {
        assert_eq!(SampleDependencyTypeEntry::new(4, 0, 0, 0), None);
        assert_eq!(SampleDependencyTypeEntry::new(0, 0, 0, 4), None);
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let payload = b"\x01\0\0\0";

        assert_eq!(
            SampleDependencyTypeBox::decode_payload(payload),
            Err(Error::unsupported_version(1))
        );
    }
}
