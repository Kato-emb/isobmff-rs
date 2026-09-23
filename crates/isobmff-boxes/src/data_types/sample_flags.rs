//! [`SampleFlags`], the `sample_flags` a `trex`, a `tfhd` and a `trun` carry, ISO/IEC 14496-12 §8.8.3.1

use isobmff_core::{Error, FieldReader};

use crate::padb::PAD_MAXIMUM;
use crate::sdtp::FIELD_MAXIMUM;
use crate::{DegradationPriorityEntry, PaddingBitsEntry, SampleDependencyTypeEntry};

/// Bits of the `sample_flags` §8.8.3.1 reserves, which are 0
const RESERVED_BITS: u32 = 0xf000_0000;

/// Bit of the `sample_flags` stating `sample_is_non_sync_sample`
const NON_SYNC_SAMPLE: u32 = 0x0001_0000;

/// The `sample_flags` of a sample, which cannot state a reserved bit
///
/// ISO/IEC 14496-12 §8.8.3.1 lays the 32 bits out as 4 reserved bits, the four
/// answers of an `sdtp` entry (§8.6.4), the padding bits of a `padb` entry
/// (§8.7.6), whether the sample is left out of the sync samples an `stss`
/// lists (§8.6.2), and the priority of an `stdp` entry (§8.5.3), so each field
/// is the entry of the table stating it. A `trex`, a `tfhd` and a `trun` carry
/// the word as [`bits`](Self::bits) returns it.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{DegradationPriorityEntry, PaddingBitsEntry, SampleDependencyTypeEntry, SampleFlags};
///
/// // A sample depending on others, left out of the sync samples
/// let sample_flags = SampleFlags::new(
///     SampleDependencyTypeEntry::new(0, 1, 0, 0).unwrap(),
///     PaddingBitsEntry::default(),
///     true,
///     DegradationPriorityEntry::default(),
/// );
/// assert_eq!(sample_flags.bits(), 0x0101_0000);
///
/// // A word setting a reserved bit states no sample flags
/// assert_eq!(SampleFlags::from_bits(0x1000_0000), None);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SampleFlags(u32);

impl SampleFlags {
    /// Flags of a sync sample whose other fields are all 0
    pub const ZERO: Self = Self(0);

    /// Creates the flags from the fields they state
    #[must_use]
    pub const fn new(
        sample_dependency_type: SampleDependencyTypeEntry,
        padding_bits: PaddingBitsEntry,
        sample_is_non_sync_sample: bool,
        degradation_priority: DegradationPriorityEntry,
    ) -> Self {
        let high =
            sample_dependency_type.is_leading() << 2 | sample_dependency_type.sample_depends_on();
        let low = sample_dependency_type.sample_is_depended_on() << 6
            | sample_dependency_type.sample_has_redundancy() << 4
            | padding_bits.pad() << 1
            | sample_is_non_sync_sample as u8;
        let [priority_high, priority_low] = degradation_priority.priority().to_be_bytes();

        Self(u32::from_be_bytes([high, low, priority_high, priority_low]))
    }

    /// Creates the flags from the word the wire carries
    ///
    /// Returns `None` when `bits` set one of the reserved bits.
    #[must_use]
    pub const fn from_bits(bits: u32) -> Option<Self> {
        if bits & RESERVED_BITS != 0 {
            return None;
        }

        Some(Self(bits))
    }

    /// Returns the word the wire carries
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Returns the answers an `sdtp` entry states for the sample
    #[must_use]
    pub fn sample_dependency_type(self) -> SampleDependencyTypeEntry {
        let [high, low, _, _] = self.0.to_be_bytes();
        // Why not unwrap: each answer is masked to its 2 bits, so the entry
        // always builds, and a degenerate value stands in for the panic the
        // lints forbid.
        SampleDependencyTypeEntry::new(
            (high >> 2) & FIELD_MAXIMUM,
            high & FIELD_MAXIMUM,
            low >> 6,
            (low >> 4) & FIELD_MAXIMUM,
        )
        .unwrap_or_default()
    }

    /// Returns the padding bits a `padb` entry states for the sample
    #[must_use]
    pub fn padding_bits(self) -> PaddingBitsEntry {
        let [_, low, _, _] = self.0.to_be_bytes();
        // Why not unwrap: the value is masked to its 3 bits, so the entry always
        // builds, and a degenerate value stands in for the panic the lints
        // forbid.
        PaddingBitsEntry::new((low >> 1) & PAD_MAXIMUM).unwrap_or_default()
    }

    /// Returns whether the sample is left out of the sync samples
    #[must_use]
    pub const fn sample_is_non_sync_sample(self) -> bool {
        self.0 & NON_SYNC_SAMPLE != 0
    }

    /// Returns the priority an `stdp` entry states for the sample
    #[must_use]
    pub fn degradation_priority(self) -> DegradationPriorityEntry {
        let [_, _, priority_high, priority_low] = self.0.to_be_bytes();

        DegradationPriorityEntry::new(u16::from_be_bytes([priority_high, priority_low]))
    }
}

/// Reads a `sample_flags` word
///
/// # Errors
///
/// * [`UnsupportedFlags`](isobmff_core::ErrorKind::UnsupportedFlags): the word
///   sets a reserved bit.
/// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
///   ends inside the word.
pub(crate) fn read_sample_flags(reader: &mut FieldReader<'_>) -> Result<SampleFlags, Error> {
    let bits = reader.read_u32()?;

    SampleFlags::from_bits(bits).ok_or(Error::unsupported_flags(bits))
}

#[cfg(test)]
mod tests {
    use super::SampleFlags;
    use crate::{DegradationPriorityEntry, PaddingBitsEntry, SampleDependencyTypeEntry};

    #[test]
    fn each_field_lies_where_the_layout_places_it() {
        let sample_dependency_type = SampleDependencyTypeEntry::new(3, 2, 1, 2).unwrap();
        let padding_bits = PaddingBitsEntry::new(5).unwrap();
        let degradation_priority = DegradationPriorityEntry::new(0xbeef);

        let sample_flags = SampleFlags::new(
            sample_dependency_type,
            padding_bits,
            true,
            degradation_priority,
        );

        assert_eq!(sample_flags.bits(), 0x0e6b_beef);
        assert_eq!(SampleFlags::from_bits(0x0e6b_beef), Some(sample_flags));
        assert_eq!(
            (
                sample_flags.sample_dependency_type(),
                sample_flags.padding_bits(),
                sample_flags.sample_is_non_sync_sample(),
                sample_flags.degradation_priority(),
            ),
            (
                sample_dependency_type,
                padding_bits,
                true,
                degradation_priority
            )
        );
    }

    #[test]
    fn a_word_setting_a_reserved_bit_states_no_sample_flags() {
        assert_eq!(SampleFlags::from_bits(0x1000_0000), None);
    }
}
