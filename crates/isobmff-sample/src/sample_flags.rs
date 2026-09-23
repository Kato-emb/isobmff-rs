//! [`SampleFlagFields`], the `sample_flags` of a sample laid out as the sample tables state them, ISO/IEC 14496-12 §8.8.3.1

use isobmff_boxes::{DegradationPriorityEntry, PaddingBitsEntry, SampleDependencyTypeEntry};

/// Mask of the 2 bits one answer of an `sdtp` entry occupies
const TWO_BITS: u8 = 0b11;

/// Mask of the 3 bits `sample_padding_value` occupies
const THREE_BITS: u8 = 0b111;

/// What the `sample_flags` of one sample state, field by field
///
/// §8.8.3.1 lays the 32 bits out as 4 reserved bits, the four answers of an
/// `sdtp` entry (§8.6.4), the padding bits of a `padb` entry (§8.7.6), whether
/// the sample is left out of the sync samples an `stss` lists (§8.6.2), and the
/// priority of an `stdp` entry (§8.5.3), so each field is the entry the table
/// stating it holds.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct SampleFlagFields {
    pub(crate) dependency: SampleDependencyTypeEntry,
    pub(crate) padding: PaddingBitsEntry,
    pub(crate) is_non_sync_sample: bool,
    pub(crate) degradation_priority: DegradationPriorityEntry,
}

impl SampleFlagFields {
    /// Splits `sample_flags` into its fields
    ///
    /// Returns `None` when one of the reserved bits is set, which no field
    /// holds.
    pub(crate) fn from_sample_flags(sample_flags: u32) -> Option<Self> {
        let [high, low, priority_high, priority_low] = sample_flags.to_be_bytes();
        if high >> 4 != 0 {
            return None;
        }

        Some(Self {
            dependency: SampleDependencyTypeEntry::new(
                (high >> 2) & TWO_BITS,
                high & TWO_BITS,
                low >> 6,
                (low >> 4) & TWO_BITS,
            )?,
            padding: PaddingBitsEntry::new((low >> 1) & THREE_BITS)?,
            is_non_sync_sample: low & 1 != 0,
            degradation_priority: DegradationPriorityEntry::new(u16::from_be_bytes([
                priority_high,
                priority_low,
            ])),
        })
    }

    /// Joins the fields into the `sample_flags` stating them
    pub(crate) fn to_sample_flags(self) -> u32 {
        let dependency = self.dependency;
        let high = dependency.is_leading() << 2 | dependency.sample_depends_on();
        let low = dependency.sample_is_depended_on() << 6
            | dependency.sample_has_redundancy() << 4
            | self.padding.pad() << 1
            | u8::from(self.is_non_sync_sample);
        let [priority_high, priority_low] = self.degradation_priority.priority().to_be_bytes();

        u32::from_be_bytes([high, low, priority_high, priority_low])
    }
}

#[cfg(test)]
mod tests {
    use isobmff_boxes::{DegradationPriorityEntry, PaddingBitsEntry, SampleDependencyTypeEntry};

    use super::SampleFlagFields;

    #[test]
    fn each_field_lies_where_the_layout_places_it() {
        let fields = SampleFlagFields {
            dependency: SampleDependencyTypeEntry::new(3, 2, 1, 2).unwrap(),
            padding: PaddingBitsEntry::new(5).unwrap(),
            is_non_sync_sample: true,
            degradation_priority: DegradationPriorityEntry::new(0xbeef),
        };

        assert_eq!(fields.to_sample_flags(), 0x0e6b_beef);
        assert_eq!(
            SampleFlagFields::from_sample_flags(0x0e6b_beef),
            Some(fields)
        );
    }

    #[test]
    fn flags_setting_a_reserved_bit_split_into_no_fields() {
        assert_eq!(SampleFlagFields::from_sample_flags(0x1000_0000), None);
    }
}
