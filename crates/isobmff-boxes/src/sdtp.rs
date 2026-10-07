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

/// Whether a sample is a leading sample, `is_leading` of ISO/IEC 14496-12 §8.6.4.3
#[allow(
    clippy::exhaustive_enums,
    reason = "ISO/IEC 14496-12 §8.6.4.3 closes the 2 bits of the field over exactly these four values"
)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub enum IsLeading {
    /// The leading nature of the sample is unknown
    #[default]
    Unknown = 0,
    /// A leading sample that depends on a sample before the referenced
    /// I-picture, and so cannot be decoded
    LeadingWithDependency = 1,
    /// Not a leading sample
    NotLeading = 2,
    /// A leading sample that depends on no sample before the referenced
    /// I-picture, and so can be decoded
    LeadingWithoutDependency = 3,
}

/// Whether a sample depends on others, `sample_depends_on` of ISO/IEC 14496-12 §8.6.4.3
#[allow(
    clippy::exhaustive_enums,
    reason = "ISO/IEC 14496-12 §8.6.4.3 closes the 2 bits of the field over exactly these four values"
)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub enum SampleDependsOn {
    /// The dependency of the sample is unknown
    #[default]
    Unknown = 0,
    /// The sample depends on others, not an I-picture
    DependsOnOthers = 1,
    /// The sample depends on no other, an I-picture
    DoesNotDependOnOthers = 2,
    /// Reserved
    Reserved = 3,
}

/// Whether other samples depend on a sample, `sample_is_depended_on` of ISO/IEC 14496-12 §8.6.4.3
#[allow(
    clippy::exhaustive_enums,
    reason = "ISO/IEC 14496-12 §8.6.4.3 closes the 2 bits of the field over exactly these four values"
)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub enum SampleIsDependedOn {
    /// Whether other samples depend on the sample is unknown
    #[default]
    Unknown = 0,
    /// Other samples may depend on the sample, which is not disposable
    NotDisposable = 1,
    /// No other sample depends on the sample, which is disposable
    Disposable = 2,
    /// Reserved
    Reserved = 3,
}

/// Whether a sample carries redundant coding, `sample_has_redundancy` of ISO/IEC 14496-12 §8.6.4.3
#[allow(
    clippy::exhaustive_enums,
    reason = "ISO/IEC 14496-12 §8.6.4.3 closes the 2 bits of the field over exactly these four values"
)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub enum SampleHasRedundancy {
    /// Whether the sample carries redundant coding is unknown
    #[default]
    Unknown = 0,
    /// The sample carries redundant coding
    RedundantCoding = 1,
    /// The sample carries no redundant coding
    NoRedundantCoding = 2,
    /// Reserved
    Reserved = 3,
}

/// Reads and writes the 2 bits a field of an entry is carried in
macro_rules! two_bits {
    ($field:ty, $zero:ident, $one:ident, $two:ident, $three:ident) => {
        impl $field {
            /// Reads the field from the low 2 bits of `bits`
            pub(crate) const fn from_bits(bits: u8) -> Self {
                match bits & FIELD_MAXIMUM {
                    0 => Self::$zero,
                    1 => Self::$one,
                    2 => Self::$two,
                    _ => Self::$three,
                }
            }

            /// Returns the 2 bits the field is carried in
            pub(crate) const fn bits(self) -> u8 {
                self as u8
            }
        }
    };
}

two_bits!(
    IsLeading,
    Unknown,
    LeadingWithDependency,
    NotLeading,
    LeadingWithoutDependency
);
two_bits!(
    SampleDependsOn,
    Unknown,
    DependsOnOthers,
    DoesNotDependOnOthers,
    Reserved
);
two_bits!(
    SampleIsDependedOn,
    Unknown,
    NotDisposable,
    Disposable,
    Reserved
);
two_bits!(
    SampleHasRedundancy,
    Unknown,
    RedundantCoding,
    NoRedundantCoding,
    Reserved
);

/// One entry of the table a [`SampleDependencyTypeBox`] holds
///
/// The entry states how the sample it is indexed by stands to the samples
/// around it, each field in the 2 bits §8.6.4 gives it.
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct SampleDependencyTypeEntry {
    is_leading: IsLeading,
    sample_depends_on: SampleDependsOn,
    sample_is_depended_on: SampleIsDependedOn,
    sample_has_redundancy: SampleHasRedundancy,
}

impl SampleDependencyTypeEntry {
    /// Creates the entry from the four answers it states for its sample
    #[must_use]
    pub const fn new(
        is_leading: IsLeading,
        sample_depends_on: SampleDependsOn,
        sample_is_depended_on: SampleIsDependedOn,
        sample_has_redundancy: SampleHasRedundancy,
    ) -> Self {
        Self {
            is_leading,
            sample_depends_on,
            sample_is_depended_on,
            sample_has_redundancy,
        }
    }

    /// Returns whether the sample is a leading sample, and whether it decodes as one
    #[must_use]
    pub const fn is_leading(&self) -> IsLeading {
        self.is_leading
    }

    /// Returns whether the sample depends on others
    #[must_use]
    pub const fn sample_depends_on(&self) -> SampleDependsOn {
        self.sample_depends_on
    }

    /// Returns whether other samples depend on this one
    #[must_use]
    pub const fn sample_is_depended_on(&self) -> SampleIsDependedOn {
        self.sample_is_depended_on
    }

    /// Returns whether the sample carries redundant coding
    #[must_use]
    pub const fn sample_has_redundancy(&self) -> SampleHasRedundancy {
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
    type Error = Error;

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
                is_leading: IsLeading::from_bits(byte >> 6),
                sample_depends_on: SampleDependsOn::from_bits(byte >> 4),
                sample_is_depended_on: SampleIsDependedOn::from_bits(byte >> 2),
                sample_has_redundancy: SampleHasRedundancy::from_bits(*byte),
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
            writer.write_bytes(&[entry.is_leading.bits() << 6
                | entry.sample_depends_on.bits() << 4
                | entry.sample_is_depended_on.bits() << 2
                | entry.sample_has_redundancy.bits()])?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::{
        IsLeading, SampleDependencyTypeBox, SampleDependencyTypeEntry, SampleDependsOn,
        SampleHasRedundancy, SampleIsDependedOn,
    };

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(sample_dependency_type: &SampleDependencyTypeBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(sample_dependency_type.payload_len()).unwrap()];
        sample_dependency_type.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let sample_dependency_type = SampleDependencyTypeBox::new(vec![
            SampleDependencyTypeEntry::new(
                IsLeading::NotLeading,
                SampleDependsOn::DoesNotDependOnOthers,
                SampleIsDependedOn::NotDisposable,
                SampleHasRedundancy::Unknown,
            ),
            SampleDependencyTypeEntry::new(
                IsLeading::LeadingWithDependency,
                SampleDependsOn::DependsOnOthers,
                SampleIsDependedOn::Disposable,
                SampleHasRedundancy::Reserved,
            ),
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
    fn every_value_of_every_field_reads_back_as_the_bits_it_came_as() {
        let payload = b"\0\0\0\0\x00\x55\xaa\xff";

        let decoded = SampleDependencyTypeBox::decode_payload(payload).unwrap();

        assert_eq!(
            decoded.entries(),
            [
                SampleDependencyTypeEntry::default(),
                SampleDependencyTypeEntry::new(
                    IsLeading::LeadingWithDependency,
                    SampleDependsOn::DependsOnOthers,
                    SampleIsDependedOn::NotDisposable,
                    SampleHasRedundancy::RedundantCoding,
                ),
                SampleDependencyTypeEntry::new(
                    IsLeading::NotLeading,
                    SampleDependsOn::DoesNotDependOnOthers,
                    SampleIsDependedOn::Disposable,
                    SampleHasRedundancy::NoRedundantCoding,
                ),
                SampleDependencyTypeEntry::new(
                    IsLeading::LeadingWithoutDependency,
                    SampleDependsOn::Reserved,
                    SampleIsDependedOn::Reserved,
                    SampleHasRedundancy::Reserved,
                ),
            ]
        );
        assert_eq!(encoded_payload(&decoded), payload);
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
