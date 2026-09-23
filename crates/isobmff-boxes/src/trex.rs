//! [`TrackExtendsBox`] (`trex`), ISO/IEC 14496-12 §8.8.3, and [`SampleFlags`], the `sample_flags` its defaults and the fragments share, §8.8.3.1

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFields,
    FullBoxFlags,
};

use crate::padb::PAD_MAXIMUM;
use crate::sdtp::FIELD_MAXIMUM;
use crate::{DegradationPriorityEntry, PaddingBitsEntry, SampleDependencyTypeEntry};

/// Length of the payload, which has no version-dependent field
const PAYLOAD_LEN: u64 = 24;

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

/// Box that sets the defaults every fragment of one track falls back on
///
/// [`TrackExtendsBox`] (`trex`), ISO/IEC 14496-12 §8.8.3. A `tfhd` or `trun`
/// that leaves a sample property unstated takes it from here, so a `mvex`
/// carries one of these per track the movie declares.
#[doc(alias = "trex")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TrackExtendsBox {
    track_id: u32,
    default_sample_description_index: u32,
    default_sample_duration: u32,
    default_sample_size: u32,
    default_sample_flags: SampleFlags,
}

impl TrackExtendsBox {
    /// Creates the box from the defaults it sets
    #[must_use]
    pub const fn new(
        track_id: u32,
        default_sample_description_index: u32,
        default_sample_duration: u32,
        default_sample_size: u32,
        default_sample_flags: SampleFlags,
    ) -> Self {
        Self {
            track_id,
            default_sample_description_index,
            default_sample_duration,
            default_sample_size,
            default_sample_flags,
        }
    }

    /// Returns the track these defaults apply to
    #[must_use]
    pub const fn track_id(&self) -> u32 {
        self.track_id
    }

    /// Returns the `stsd` entry a sample of this track is described by
    #[must_use]
    pub const fn default_sample_description_index(&self) -> u32 {
        self.default_sample_description_index
    }

    /// Returns how long a sample of this track lasts, in the media time scale
    #[must_use]
    pub const fn default_sample_duration(&self) -> u32 {
        self.default_sample_duration
    }

    /// Returns how many bytes a sample of this track occupies
    #[must_use]
    pub const fn default_sample_size(&self) -> u32 {
        self.default_sample_size
    }

    /// Returns the sample flags a sample of this track carries
    #[must_use]
    pub const fn default_sample_flags(&self) -> SampleFlags {
        self.default_sample_flags
    }
}

impl BoxDefinition for TrackExtendsBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"trex");
}

impl BoxDecode for TrackExtendsBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0.
    /// * [`UnsupportedFlags`](isobmff_core::ErrorKind::UnsupportedFlags): the
    ///   `default_sample_flags` set a bit §8.8.3.1 reserves.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
    ///   ends inside a field of the box.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version != 0 {
            return Err(Error::unsupported_version(version));
        }

        let track_id = reader.read_u32()?;
        let default_sample_description_index = reader.read_u32()?;
        let default_sample_duration = reader.read_u32()?;
        let default_sample_size = reader.read_u32()?;
        let default_sample_flags = read_sample_flags(reader)?;

        Ok(Self {
            track_id,
            default_sample_description_index,
            default_sample_duration,
            default_sample_size,
            default_sample_flags,
        })
    }
}

impl BoxEncode for TrackExtendsBox {
    fn payload_len(&self) -> u64 {
        PAYLOAD_LEN
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxFields::new(0, FullBoxFlags::ZERO).to_bytes())?;
        writer.write_u32(self.track_id)?;
        writer.write_u32(self.default_sample_description_index)?;
        writer.write_u32(self.default_sample_duration)?;
        writer.write_u32(self.default_sample_size)?;
        writer.write_u32(self.default_sample_flags.bits())?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::{SampleFlags, TrackExtendsBox};
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

    #[test]
    fn default_sample_flags_setting_a_reserved_bit_are_rejected() {
        let mut payload = vec![0; 24];
        *payload.get_mut(20).unwrap() = 0x80;

        assert_eq!(
            TrackExtendsBox::decode_payload(&payload),
            Err(Error::unsupported_flags(0x8000_0000))
        );
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let track_extends = TrackExtendsBox::new(
            1,
            1,
            1_024,
            0,
            SampleFlags::new(
                SampleDependencyTypeEntry::default(),
                PaddingBitsEntry::default(),
                true,
                DegradationPriorityEntry::default(),
            ),
        );
        let mut payload = vec![0; 24];

        track_extends.encode_payload(&mut payload).unwrap();

        assert_eq!(
            TrackExtendsBox::decode_payload(&payload).unwrap(),
            track_extends
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let mut payload = vec![0; 24];
        *payload.first_mut().unwrap() = 1;

        assert_eq!(
            TrackExtendsBox::decode_payload(&payload),
            Err(Error::unsupported_version(1))
        );
    }

    #[test]
    fn a_payload_shorter_than_the_fields_is_rejected() {
        assert_eq!(
            TrackExtendsBox::decode_payload(&[0; 23]),
            Err(Error::truncated_payload(24, 23))
        );
    }
}
