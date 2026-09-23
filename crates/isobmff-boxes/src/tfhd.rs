//! [`TrackFragmentHeaderBox`] (`tfhd`) and [`TrackFragmentHeaderFlags`], the `tf_flags` a caller states, ISO/IEC 14496-12 §8.8.7

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFields,
    FullBoxFlags,
};

use crate::trex::{SampleFlags, read_sample_flags};

/// Length of the fields every fragment header carries
const FIXED_FIELDS_LEN: u64 = 8;

/// Length of the `base_data_offset` field, the one optional field of 64 bits
const BASE_DATA_OFFSET_LEN: u64 = 8;

/// Length of every optional field other than `base_data_offset`
const OPTIONAL_FIELD_LEN: u64 = 4;

/// Flag stating that `base_data_offset` is present
const BASE_DATA_OFFSET_PRESENT: u32 = 0x0000_0001;

/// Flag stating that `sample_description_index` is present
const SAMPLE_DESCRIPTION_INDEX_PRESENT: u32 = 0x0000_0002;

/// Flag stating that `default_sample_duration` is present
const DEFAULT_SAMPLE_DURATION_PRESENT: u32 = 0x0000_0008;

/// Flag stating that `default_sample_size` is present
const DEFAULT_SAMPLE_SIZE_PRESENT: u32 = 0x0000_0010;

/// Flag stating that `default_sample_flags` is present
const DEFAULT_SAMPLE_FLAGS_PRESENT: u32 = 0x0000_0020;

/// Every flag stating that a field of this box is present
const PRESENCE_FLAGS: u32 = BASE_DATA_OFFSET_PRESENT
    | SAMPLE_DESCRIPTION_INDEX_PRESENT
    | DEFAULT_SAMPLE_DURATION_PRESENT
    | DEFAULT_SAMPLE_SIZE_PRESENT
    | DEFAULT_SAMPLE_FLAGS_PRESENT;

/// Flag stating that the fragment of the box holds no samples
const DURATION_IS_EMPTY: u32 = 0x0001_0000;

/// Flag stating that the data offsets of the fragment are anchored at the `moof`
const DEFAULT_BASE_IS_MOOF: u32 = 0x0002_0000;

/// The `tf_flags` of a `tfhd` that no field of the box and no rule of its `traf` speaks for
///
/// ISO/IEC 14496-12 §8.8.7.1 has the `tf_flags` state which optional fields
/// the box carries, which a [`TrackFragmentHeaderBox`] derives from the fields
/// themselves, and `duration-is-empty`, which
/// [`TrackFragmentBox`](crate::TrackFragmentBox) states for the fragment it
/// builds. What is left for a caller to state is
/// [`default-base-is-moof`](Self::DEFAULT_BASE_IS_MOOF) and whatever bits the
/// spec has yet to define, which this holds alone and
/// [`TrackFragmentHeaderBox::new`] takes.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::TrackFragmentHeaderFlags;
///
/// // A bit stating a field, or the emptiness a `traf` states, is refused
/// assert_eq!(TrackFragmentHeaderFlags::new(0x0000_0008), None);
/// assert_eq!(TrackFragmentHeaderFlags::new(0x0001_0000), None);
///
/// // A bit the spec has yet to define is carried
/// assert_eq!(TrackFragmentHeaderFlags::new(0x0004_0000).map(TrackFragmentHeaderFlags::bits), Some(0x0004_0000));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TrackFragmentHeaderFlags(u32);

impl TrackFragmentHeaderFlags {
    /// No flag at all
    pub const ZERO: Self = Self(0);

    /// Flag stating that the data offsets of the fragment are anchored at the `moof`
    pub const DEFAULT_BASE_IS_MOOF: Self = Self(DEFAULT_BASE_IS_MOOF);

    /// Creates the flags from the bits they carry
    ///
    /// Returns `None` when `bits` reach past the 24 bits of the field, state
    /// that a field is present, or state `duration-is-empty`.
    #[must_use]
    pub const fn new(bits: u32) -> Option<Self> {
        if bits & (PRESENCE_FLAGS | DURATION_IS_EMPTY) != 0 || FullBoxFlags::new(bits).is_none() {
            return None;
        }

        Some(Self(bits))
    }

    /// Returns the bits the flags carry
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }
}

/// Box that sets up what the runs of one track fragment share
///
/// [`TrackFragmentHeaderBox`] (`tfhd`), ISO/IEC 14496-12 §8.8.7. Every field
/// past the `track_id` is optional, and one left out leaves the runs of this
/// fragment falling back on the default the `trex` of the track sets. A `traf`
/// carries exactly one.
///
/// Five of the `flags` state which of those fields the box carries, so they are
/// derived from the fields themselves; `duration-is-empty` is stated by
/// [`TrackFragmentBox::with_empty_duration`](crate::TrackFragmentBox::with_empty_duration);
/// and what a caller states is a [`TrackFragmentHeaderFlags`] —
/// [`default-base-is-moof`](TrackFragmentHeaderFlags::DEFAULT_BASE_IS_MOOF) and
/// whatever bits the spec has yet to define. [`flags`](Self::flags) returns all
/// of them together, as the wire carries them.
///
/// A bit the spec has yet to define is carried through, but a payload holding
/// the field such a bit would speak for is not: the fields this box reads stop
/// short of the end of the payload, which
/// [`decode_payload`](BoxDecode::decode_payload) refuses as trailing bytes.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{TrackFragmentHeaderFlags, TrackFragmentHeaderBox};
/// use isobmff_core::FullBoxFlags;
///
/// // A fragment of track 1 whose samples last 1024 units unless a run says otherwise
/// let track_fragment_header =
///     TrackFragmentHeaderBox::new(TrackFragmentHeaderFlags::ZERO, 1, None, None, Some(1_024), None, None);
///
/// // The flags state the one optional field the box was given
/// assert_eq!(
///     track_fragment_header.flags(),
///     FullBoxFlags::new(0x0000_0008).unwrap()
/// );
/// ```
#[doc(alias = "tfhd")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TrackFragmentHeaderBox {
    flags: FullBoxFlags,
    track_id: u32,
    base_data_offset: Option<u64>,
    sample_description_index: Option<u32>,
    default_sample_duration: Option<u32>,
    default_sample_size: Option<u32>,
    default_sample_flags: Option<SampleFlags>,
}

impl TrackFragmentHeaderBox {
    /// Creates the box from the track it heads and the defaults it sets
    ///
    /// The flags stating which fields are present are derived from the fields
    /// themselves, and stand beside `flags` in what [`flags`](Self::flags)
    /// returns.
    #[must_use]
    pub const fn new(
        flags: TrackFragmentHeaderFlags,
        track_id: u32,
        base_data_offset: Option<u64>,
        sample_description_index: Option<u32>,
        default_sample_duration: Option<u32>,
        default_sample_size: Option<u32>,
        default_sample_flags: Option<SampleFlags>,
    ) -> Self {
        let bits = flags.bits()
            | presence(base_data_offset.is_some(), BASE_DATA_OFFSET_PRESENT)
            | presence(
                sample_description_index.is_some(),
                SAMPLE_DESCRIPTION_INDEX_PRESENT,
            )
            | presence(
                default_sample_duration.is_some(),
                DEFAULT_SAMPLE_DURATION_PRESENT,
            )
            | presence(default_sample_size.is_some(), DEFAULT_SAMPLE_SIZE_PRESENT)
            | presence(default_sample_flags.is_some(), DEFAULT_SAMPLE_FLAGS_PRESENT);

        Self {
            flags: flags_of(bits),
            track_id,
            base_data_offset,
            sample_description_index,
            default_sample_duration,
            default_sample_size,
            default_sample_flags,
        }
    }

    /// Returns the box with `duration-is-empty` added to its flags
    pub(crate) const fn with_empty_duration(mut self) -> Self {
        self.flags = flags_of(self.flags.bits() | DURATION_IS_EMPTY);

        self
    }

    /// Returns the flags of the box, both those stating a field and those not
    #[must_use]
    pub const fn flags(&self) -> FullBoxFlags {
        self.flags
    }

    /// Returns whether the fragment states that it holds no samples
    ///
    /// A `traf` read by [`decode_payload`](BoxDecode::decode_payload) of
    /// [`TrackFragmentBox`](crate::TrackFragmentBox) is refused when it states
    /// this alongside a `trun`.
    #[must_use]
    pub const fn duration_is_empty(&self) -> bool {
        self.flags.bits() & DURATION_IS_EMPTY != 0
    }

    /// Returns whether the data offsets of this fragment are anchored at the `moof`
    ///
    /// §8.8.7.1 has this flag ignored where
    /// [`base_data_offset`](Self::base_data_offset) is given, which anchors them
    /// explicitly instead.
    #[must_use]
    pub const fn default_base_is_moof(&self) -> bool {
        self.flags.bits() & DEFAULT_BASE_IS_MOOF != 0
    }

    /// Returns the track this fragment carries samples of
    #[must_use]
    pub const fn track_id(&self) -> u32 {
        self.track_id
    }

    /// Returns the offset the data offsets of every run of this fragment count from
    #[must_use]
    pub const fn base_data_offset(&self) -> Option<u64> {
        self.base_data_offset
    }

    /// Returns the `stsd` entry the samples of this fragment are described by
    #[must_use]
    pub const fn sample_description_index(&self) -> Option<u32> {
        self.sample_description_index
    }

    /// Returns how long a sample of this fragment lasts, in the media time scale
    #[must_use]
    pub const fn default_sample_duration(&self) -> Option<u32> {
        self.default_sample_duration
    }

    /// Returns how many bytes a sample of this fragment occupies
    #[must_use]
    pub const fn default_sample_size(&self) -> Option<u32> {
        self.default_sample_size
    }

    /// Returns the sample flags a sample of this fragment carries
    #[must_use]
    pub const fn default_sample_flags(&self) -> Option<SampleFlags> {
        self.default_sample_flags
    }
}

/// Returns `flag` when the field it states is `present`, and no bit otherwise
const fn presence(present: bool, flag: u32) -> u32 {
    if present { flag } else { 0 }
}

/// Returns `bits` as the flags of the box, which they fit by construction
const fn flags_of(bits: u32) -> FullBoxFlags {
    // Why not FullBoxFlags::new: it answers with an `Option` for bits past the
    // field, which flags that already fit it, joined with bits the field
    // defines, cannot reach; the field is built from its bytes instead.
    FullBoxFields::from_bytes(&bits.to_be_bytes()).flags()
}

impl BoxDefinition for TrackFragmentHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tfhd");
}

impl BoxDecode for TrackFragmentHeaderBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0.
    /// * [`UnsupportedFlags`](isobmff_core::ErrorKind::UnsupportedFlags): the
    ///   `default_sample_flags` set a bit ISO/IEC 14496-12 §8.8.3.1 reserves.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
    ///   ends inside a field the flags state.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let full_box = FullBoxFields::from_bytes(reader.read_bytes::<4>()?);
        let version = full_box.version();
        if version != 0 {
            return Err(Error::unsupported_version(version));
        }

        let flags = full_box.flags();
        let carries = |flag: u32| flags.bits() & flag != 0;

        let track_id = reader.read_u32()?;
        let base_data_offset = if carries(BASE_DATA_OFFSET_PRESENT) {
            Some(reader.read_u64()?)
        } else {
            None
        };
        let sample_description_index = if carries(SAMPLE_DESCRIPTION_INDEX_PRESENT) {
            Some(reader.read_u32()?)
        } else {
            None
        };
        let default_sample_duration = if carries(DEFAULT_SAMPLE_DURATION_PRESENT) {
            Some(reader.read_u32()?)
        } else {
            None
        };
        let default_sample_size = if carries(DEFAULT_SAMPLE_SIZE_PRESENT) {
            Some(reader.read_u32()?)
        } else {
            None
        };
        let default_sample_flags = if carries(DEFAULT_SAMPLE_FLAGS_PRESENT) {
            Some(read_sample_flags(reader)?)
        } else {
            None
        };

        Ok(Self {
            flags,
            track_id,
            base_data_offset,
            sample_description_index,
            default_sample_duration,
            default_sample_size,
            default_sample_flags,
        })
    }
}

impl BoxEncode for TrackFragmentHeaderBox {
    fn payload_len(&self) -> u64 {
        let mut length = FIXED_FIELDS_LEN;
        if self.base_data_offset.is_some() {
            length = length.saturating_add(BASE_DATA_OFFSET_LEN);
        }
        for present in [
            self.sample_description_index.is_some(),
            self.default_sample_duration.is_some(),
            self.default_sample_size.is_some(),
            self.default_sample_flags.is_some(),
        ] {
            if present {
                length = length.saturating_add(OPTIONAL_FIELD_LEN);
            }
        }

        length
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxFields::new(0, self.flags).to_bytes())?;
        writer.write_u32(self.track_id)?;
        if let Some(base_data_offset) = self.base_data_offset {
            writer.write_u64(base_data_offset)?;
        }
        for field in [
            self.sample_description_index,
            self.default_sample_duration,
            self.default_sample_size,
            self.default_sample_flags.map(SampleFlags::bits),
        ]
        .into_iter()
        .flatten()
        {
            writer.write_u32(field)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error, FullBoxFlags};

    use super::{TrackFragmentHeaderBox, TrackFragmentHeaderFlags};
    use crate::{
        DegradationPriorityEntry, PaddingBitsEntry, SampleDependencyTypeEntry, SampleFlags,
    };

    /// Fragment header carrying every optional field the box defines
    fn every_field() -> TrackFragmentHeaderBox {
        TrackFragmentHeaderBox::new(
            TrackFragmentHeaderFlags::ZERO,
            1,
            Some(4_096),
            Some(1),
            Some(1_024),
            Some(512),
            Some(SampleFlags::new(
                SampleDependencyTypeEntry::new(0, 1, 0, 0).unwrap(),
                PaddingBitsEntry::default(),
                false,
                DegradationPriorityEntry::default(),
            )),
        )
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(track_fragment_header: &TrackFragmentHeaderBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(track_fragment_header.payload_len()).unwrap()];
        track_fragment_header.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_carrying_every_optional_field_reads_back_as_the_value_that_wrote_it() {
        let payload = encoded_payload(&every_field());

        assert_eq!(
            TrackFragmentHeaderBox::decode_payload(&payload).unwrap(),
            every_field()
        );
    }

    #[test]
    fn a_box_carrying_no_optional_field_reads_back_as_the_value_that_wrote_it() {
        let track_fragment_header = TrackFragmentHeaderBox::new(
            TrackFragmentHeaderFlags::ZERO,
            1,
            None,
            None,
            None,
            None,
            None,
        );

        let payload = encoded_payload(&track_fragment_header);

        assert_eq!(payload, b"\0\0\0\0\0\0\0\x01");
        assert_eq!(
            TrackFragmentHeaderBox::decode_payload(&payload).unwrap(),
            track_fragment_header
        );
    }

    #[test]
    fn the_flags_state_which_optional_fields_the_box_carries() {
        let payload = encoded_payload(&every_field());

        assert_eq!(payload.get(..4), Some(b"\0\0\0\x3b".as_slice()));
    }

    #[test]
    fn the_flags_no_field_speaks_for_stand_beside_the_flags_that_state_a_field() {
        let track_fragment_header = TrackFragmentHeaderBox::new(
            TrackFragmentHeaderFlags::DEFAULT_BASE_IS_MOOF,
            1,
            None,
            None,
            Some(1_024),
            None,
            None,
        );

        assert_eq!(
            track_fragment_header.flags(),
            FullBoxFlags::new(0x0002_0008).unwrap()
        );
    }

    #[test]
    fn a_fragment_anchored_at_the_movie_fragment_says_so() {
        let track_fragment_header = TrackFragmentHeaderBox::new(
            TrackFragmentHeaderFlags::DEFAULT_BASE_IS_MOOF,
            1,
            None,
            None,
            None,
            None,
            None,
        );

        assert!(track_fragment_header.default_base_is_moof());
        assert!(!every_field().default_base_is_moof());
    }

    #[test]
    fn a_flag_the_spec_has_yet_to_define_is_carried_through() {
        let undefined = TrackFragmentHeaderFlags::new(0x0004_0000).unwrap();
        let track_fragment_header =
            TrackFragmentHeaderBox::new(undefined, 1, None, None, None, None, None);

        let payload = encoded_payload(&track_fragment_header);

        assert_eq!(
            TrackFragmentHeaderBox::decode_payload(&payload).unwrap(),
            track_fragment_header
        );
    }

    #[test]
    fn flags_stating_a_field_an_empty_duration_or_a_bit_past_the_field_cannot_be_built() {
        assert_eq!(TrackFragmentHeaderFlags::new(0x0000_0002), None);
        assert_eq!(TrackFragmentHeaderFlags::new(0x0001_0000), None);
        assert_eq!(TrackFragmentHeaderFlags::new(0x0100_0000), None);
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let mut payload = encoded_payload(&every_field());
        *payload.first_mut().unwrap() = 1;

        assert_eq!(
            TrackFragmentHeaderBox::decode_payload(&payload),
            Err(Error::unsupported_version(1))
        );
    }

    #[test]
    fn a_payload_holding_a_field_no_flag_states_is_rejected() {
        let mut payload = encoded_payload(&every_field());
        payload.extend_from_slice(&[0; 4]);

        assert_eq!(
            TrackFragmentHeaderBox::decode_payload(&payload),
            Err(Error::trailing_payload(32, 36))
        );
    }

    #[test]
    fn default_sample_flags_setting_a_reserved_bit_are_rejected() {
        let mut payload = encoded_payload(&every_field());
        *payload.get_mut(28).unwrap() = 0x10;

        assert_eq!(
            TrackFragmentHeaderBox::decode_payload(&payload),
            Err(Error::unsupported_flags(0x1000_0000))
        );
    }

    #[test]
    fn a_payload_shorter_than_the_fields_its_flags_state_is_rejected() {
        let payload = encoded_payload(&every_field());

        assert_eq!(
            TrackFragmentHeaderBox::decode_payload(payload.get(..27).unwrap()),
            Err(Error::truncated_payload(28, 27))
        );
    }
}
