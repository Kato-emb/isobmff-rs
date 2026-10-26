//! [`SampleFlags`], the flags of a sample in a movie fragment packed into 32 bits

use isobmff_core::Error;

/// Flags of a sample in a movie fragment, ISO/IEC 14496-12 §8.8.3.1
///
/// The flags a `default_sample_flags`, `first_sample_flags` or `sample_flags`
/// field packs into 32 bits. Each field is held as the integer it packs:
/// `is_leading`, `sample_depends_on`, `sample_is_depended_on` and
/// `sample_has_redundancy` take the values of the
/// [`SampleDependencyTypeEntry`](crate::SampleDependencyTypeEntry), and
/// `sample_padding_value` and `sample_degradation_priority` those of the
/// `PaddingBitsBox` and the `DegradationPriorityBox`. The four `reserved`
/// bits are read past and written as zero.
///
/// [`Default`] is the flags whose fields are all zero.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::SampleFlags;
///
/// // The flags of a sample that depends on others and is not a sync sample
/// let flags = SampleFlags::from_bits(0x0101_0000);
/// assert_eq!(flags.sample_depends_on, 1);
/// assert!(flags.sample_is_non_sync_sample);
/// assert_eq!(flags.to_bits(), Ok(0x0101_0000));
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct SampleFlags {
    /// 2-bit value: 1 for a leading sample that cannot be decoded, 2 for a
    /// sample that is not leading, 3 for a leading sample that can be decoded,
    /// 0 where it is unknown
    pub is_leading: u8,
    /// 2-bit value: 1 where the sample depends on others, 2 where it does not,
    /// 0 where it is unknown
    pub sample_depends_on: u8,
    /// 2-bit value: 1 where other samples may depend on this one, 2 where none
    /// does, 0 where it is unknown
    pub sample_is_depended_on: u8,
    /// 2-bit value: 1 where the sample holds redundant coding, 2 where it holds
    /// none, 0 where it is unknown
    pub sample_has_redundancy: u8,
    /// 3-bit number of bits of padding at the end of the sample
    pub sample_padding_value: u8,
    /// Whether the sample is not a sync sample
    pub sample_is_non_sync_sample: bool,
    /// Degradation priority of the sample
    pub sample_degradation_priority: u16,
}

impl SampleFlags {
    /// Creates the flags from the 32 bits a field packs them into, dropping the `reserved` bits
    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        let [high, low, priority_high, priority_low] = bits.to_be_bytes();

        Self {
            is_leading: high >> 2 & 0b11,
            sample_depends_on: high & 0b11,
            sample_is_depended_on: low >> 6,
            sample_has_redundancy: low >> 4 & 0b11,
            sample_padding_value: low >> 1 & 0b111,
            sample_is_non_sync_sample: low & 1 != 0,
            sample_degradation_priority: u16::from_be_bytes([priority_high, priority_low]),
        }
    }

    /// Returns the 32 bits a field packs the flags into, the `reserved` bits zero
    ///
    /// # Errors
    ///
    /// * [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue): a field
    ///   holds a value wider than its bits, named by the field.
    pub fn to_bits(self) -> Result<u32, Error> {
        let fields = [
            (self.is_leading, 0b11, "is_leading"),
            (self.sample_depends_on, 0b11, "sample_depends_on"),
            (self.sample_is_depended_on, 0b11, "sample_is_depended_on"),
            (self.sample_has_redundancy, 0b11, "sample_has_redundancy"),
            (self.sample_padding_value, 0b111, "sample_padding_value"),
        ];
        if let Some((_, _, field)) = fields.iter().find(|(value, widest, _)| value > widest) {
            return Err(Error::invalid_value(field));
        }

        Ok(u32::from_be_bytes([
            self.is_leading << 2 | self.sample_depends_on,
            self.sample_is_depended_on << 6
                | self.sample_has_redundancy << 4
                | self.sample_padding_value << 1
                | u8::from(self.sample_is_non_sync_sample),
            0,
            0,
        ]) | u32::from(self.sample_degradation_priority))
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::Error;

    use super::SampleFlags;

    fn every_field_set() -> SampleFlags {
        SampleFlags {
            is_leading: 3,
            sample_depends_on: 2,
            sample_is_depended_on: 1,
            sample_has_redundancy: 2,
            sample_padding_value: 5,
            sample_is_non_sync_sample: true,
            sample_degradation_priority: 0x1234,
        }
    }

    #[test]
    fn each_field_is_unpacked_from_the_bits_its_syntax_lays_out() {
        assert_eq!(
            SampleFlags::from_bits(0b0000_1110_0110_1011 << 16 | 0x1234),
            every_field_set()
        );
    }

    #[test]
    fn each_field_is_packed_into_the_bits_its_syntax_lays_out() {
        assert_eq!(
            every_field_set().to_bits(),
            Ok(0b0000_1110_0110_1011 << 16 | 0x1234)
        );
    }

    #[test]
    fn the_reserved_bits_are_read_past_and_written_as_zero() {
        assert_eq!(
            SampleFlags::from_bits(0xf001_0000).to_bits(),
            Ok(0x0001_0000)
        );
    }

    #[test]
    fn a_value_wider_than_its_bits_is_refused() {
        let mut wide = every_field_set();
        wide.sample_padding_value = 8;

        assert_eq!(
            wide.to_bits(),
            Err(Error::invalid_value("sample_padding_value"))
        );
    }
}
