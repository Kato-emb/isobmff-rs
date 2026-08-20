//! [`TrackFragmentHeaderBox`] (`tfhd`), ISO/IEC 14496-12 §8.8.7

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFields,
    FullBoxFlags,
};

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

/// Flag stating that the fragment of this box holds no samples
const DURATION_IS_EMPTY: u32 = 0x0001_0000;

/// Every flag stating that a field of this box is present
const PRESENCE_FLAGS: u32 = BASE_DATA_OFFSET_PRESENT
    | SAMPLE_DESCRIPTION_INDEX_PRESENT
    | DEFAULT_SAMPLE_DURATION_PRESENT
    | DEFAULT_SAMPLE_SIZE_PRESENT
    | DEFAULT_SAMPLE_FLAGS_PRESENT;

/// Box that sets up what the runs of one track fragment share
///
/// [`TrackFragmentHeaderBox`] (`tfhd`), ISO/IEC 14496-12 §8.8.7. Every field
/// past the `track_id` is optional, and one left out leaves the runs of this
/// fragment falling back on the default the `trex` of the track sets. A `traf`
/// carries exactly one.
///
/// Five of the `flags` state which of those fields the box carries, so they are
/// derived from the fields themselves. What is held is the rest —
/// `duration-is-empty`, `default-base-is-moof`, and whatever bits the spec has
/// yet to define — and [`flags`](Self::flags) returns all of them together, as
/// the wire carries them.
///
/// A bit the spec has yet to define is carried through, but a payload holding
/// the field such a bit would speak for is not: the fields this box reads stop
/// short of the end of the payload, which
/// [`decode_payload`](BoxDecode::decode_payload) refuses as trailing bytes.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::TrackFragmentHeaderBox;
/// use isobmff_core::FullBoxFlags;
///
/// // A fragment of track 1 whose samples last 1024 units unless a run says otherwise
/// let track_fragment_header =
///     TrackFragmentHeaderBox::new(FullBoxFlags::ZERO, 1, None, None, Some(1_024), None, None)
///         .unwrap();
///
/// // The flags state the one optional field the box was given
/// assert_eq!(
///     track_fragment_header.flags(),
///     FullBoxFlags::new(0x0000_0008).unwrap()
/// );
///
/// // Stating a field in the flags without giving it builds nothing
/// assert_eq!(
///     TrackFragmentHeaderBox::new(
///         FullBoxFlags::new(0x0000_0008).unwrap(),
///         1,
///         None,
///         None,
///         None,
///         None,
///         None
///     ),
///     None
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
    default_sample_flags: Option<u32>,
}

impl TrackFragmentHeaderBox {
    /// Creates the box from the track it heads and the defaults it sets
    ///
    /// The flags stating which fields are present are derived from the fields
    /// themselves, so `flags` carries only what no field speaks for.
    ///
    /// Returns `None` when `flags` states that a field is present, which would
    /// set the flags apart from the fields they speak for.
    #[must_use]
    pub fn new(
        flags: FullBoxFlags,
        track_id: u32,
        base_data_offset: Option<u64>,
        sample_description_index: Option<u32>,
        default_sample_duration: Option<u32>,
        default_sample_size: Option<u32>,
        default_sample_flags: Option<u32>,
    ) -> Option<Self> {
        // Why not taking the flags whole, as the `tkhd` does: the bits stating a
        // field is present would stand apart from the fields they speak for, and
        // a box could declare a field it does not carry.
        if flags.bits() & PRESENCE_FLAGS != 0 {
            return None;
        }

        let bits = flags.bits()
            | base_data_offset.map_or(0, |_| BASE_DATA_OFFSET_PRESENT)
            | sample_description_index.map_or(0, |_| SAMPLE_DESCRIPTION_INDEX_PRESENT)
            | default_sample_duration.map_or(0, |_| DEFAULT_SAMPLE_DURATION_PRESENT)
            | default_sample_size.map_or(0, |_| DEFAULT_SAMPLE_SIZE_PRESENT)
            | default_sample_flags.map_or(0, |_| DEFAULT_SAMPLE_FLAGS_PRESENT);

        // Why not unwrap: the presence bits lie in the low three bytes, so
        // setting them in flags that already fit the field cannot take it out of
        // range, and this `?` stands for a `None` the call does not reach.
        let flags = FullBoxFlags::new(bits)?;

        Some(Self {
            flags,
            track_id,
            base_data_offset,
            sample_description_index,
            default_sample_duration,
            default_sample_size,
            default_sample_flags,
        })
    }

    /// Returns the flags of the box, both those stating a field and those not
    #[must_use]
    pub const fn flags(&self) -> FullBoxFlags {
        self.flags
    }

    /// Returns whether the fragment states that it holds no samples
    ///
    /// [`TrackFragmentBox`](crate::TrackFragmentBox) refuses this alongside a
    /// `trun`.
    #[must_use]
    pub const fn duration_is_empty(&self) -> bool {
        self.flags.bits() & DURATION_IS_EMPTY != 0
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
    pub const fn default_sample_flags(&self) -> Option<u32> {
        self.default_sample_flags
    }
}

impl BoxDefinition for TrackFragmentHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tfhd");
}

impl BoxDecode for TrackFragmentHeaderBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload) or
    ///   [`TrailingPayload`](isobmff_core::ErrorKind::TrailingPayload): the payload ends inside a
    ///   field the flags state, or holds bytes past the fields they state.
    fn decode_payload(payload: &[u8]) -> Result<Self, Error> {
        let mut reader = FieldReader::new(payload);
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
            Some(reader.read_u32()?)
        } else {
            None
        };
        reader.finish()?;

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

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), Error> {
        let expected = self.payload_len();
        let actual = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        if actual != expected {
            return Err(Error::buffer_length_mismatch(expected, actual));
        }

        let mut writer = FieldWriter::new(buffer);
        writer.write_bytes(&FullBoxFields::new(0, self.flags).to_bytes())?;
        writer.write_u32(self.track_id)?;
        if let Some(base_data_offset) = self.base_data_offset {
            writer.write_u64(base_data_offset)?;
        }
        for field in [
            self.sample_description_index,
            self.default_sample_duration,
            self.default_sample_size,
            self.default_sample_flags,
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

    use super::TrackFragmentHeaderBox;

    /// Fragment header carrying every optional field the box defines
    fn every_field() -> TrackFragmentHeaderBox {
        TrackFragmentHeaderBox::new(
            FullBoxFlags::ZERO,
            1,
            Some(4_096),
            Some(1),
            Some(1_024),
            Some(512),
            Some(0x0100_0000),
        )
        .unwrap()
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
        let track_fragment_header =
            TrackFragmentHeaderBox::new(FullBoxFlags::ZERO, 1, None, None, None, None, None)
                .unwrap();

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
        let anchored_at_the_movie_fragment = FullBoxFlags::new(0x0002_0000).unwrap();

        let track_fragment_header = TrackFragmentHeaderBox::new(
            anchored_at_the_movie_fragment,
            1,
            None,
            None,
            Some(1_024),
            None,
            None,
        )
        .unwrap();

        assert_eq!(
            track_fragment_header.flags(),
            FullBoxFlags::new(0x0002_0008).unwrap()
        );
    }

    #[test]
    fn a_flag_the_spec_has_yet_to_define_is_carried_through() {
        let undefined = FullBoxFlags::new(0x0004_0000).unwrap();
        let track_fragment_header =
            TrackFragmentHeaderBox::new(undefined, 1, None, None, None, None, None).unwrap();

        let payload = encoded_payload(&track_fragment_header);

        assert_eq!(
            TrackFragmentHeaderBox::decode_payload(&payload).unwrap(),
            track_fragment_header
        );
    }

    #[test]
    fn a_box_whose_flags_state_a_field_it_was_not_given_cannot_be_built() {
        assert_eq!(
            TrackFragmentHeaderBox::new(
                FullBoxFlags::new(0x0000_0002).unwrap(),
                1,
                None,
                None,
                None,
                None,
                None
            ),
            None
        );
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
    fn a_payload_shorter_than_the_fields_its_flags_state_is_rejected() {
        let payload = encoded_payload(&every_field());

        assert_eq!(
            TrackFragmentHeaderBox::decode_payload(payload.get(..27).unwrap()),
            Err(Error::truncated_payload(28, 27))
        );
    }
}
