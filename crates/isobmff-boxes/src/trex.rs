//! [`TrackExtendsBox`] (`trex`), ISO/IEC 14496-12 §8.8.3

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, DecodeError, EncodeError, FullBoxFields,
    FullBoxFlags,
};

use crate::field::{check_payload_len, split_field, split_field_mut};

/// Length of the payload, which has no version-dependent field
const PAYLOAD_LEN: u64 = 24;

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
    default_sample_flags: u32,
}

impl TrackExtendsBox {
    /// Creates the box from the defaults it sets
    #[must_use]
    pub const fn new(
        track_id: u32,
        default_sample_description_index: u32,
        default_sample_duration: u32,
        default_sample_size: u32,
        default_sample_flags: u32,
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
    pub const fn default_sample_flags(&self) -> u32 {
        self.default_sample_flags
    }
}

impl BoxDefinition for TrackExtendsBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"trex");
}

impl BoxDecode for TrackExtendsBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](DecodeError::UnsupportedVersion): the box
    ///   declares a version other than 0.
    /// * [`TruncatedPayload`](DecodeError::TruncatedPayload): the payload is
    ///   shorter than the fields require.
    /// * [`TrailingBytes`](DecodeError::TrailingBytes): the payload is longer.
    fn decode_payload(payload: &[u8]) -> Result<Self, DecodeError> {
        let available = u64::try_from(payload.len()).unwrap_or(u64::MAX);
        let (full_box_field, rest) = split_field::<4>(payload, 4, available)?;

        let version = FullBoxFields::from_bytes(full_box_field).version();
        if version != 0 {
            return Err(DecodeError::UnsupportedVersion(version));
        }
        check_payload_len(PAYLOAD_LEN, available)?;

        let (track_id, rest) = split_field::<4>(rest, PAYLOAD_LEN, available)?;
        let (description_index, rest) = split_field::<4>(rest, PAYLOAD_LEN, available)?;
        let (duration, rest) = split_field::<4>(rest, PAYLOAD_LEN, available)?;
        let (size, rest) = split_field::<4>(rest, PAYLOAD_LEN, available)?;
        let (flags, _rest) = split_field::<4>(rest, PAYLOAD_LEN, available)?;

        Ok(Self {
            track_id: u32::from_be_bytes(*track_id),
            default_sample_description_index: u32::from_be_bytes(*description_index),
            default_sample_duration: u32::from_be_bytes(*duration),
            default_sample_size: u32::from_be_bytes(*size),
            default_sample_flags: u32::from_be_bytes(*flags),
        })
    }
}

impl BoxEncode for TrackExtendsBox {
    fn payload_len(&self) -> u64 {
        PAYLOAD_LEN
    }

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), EncodeError> {
        let actual = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        let mismatch = EncodeError::BufferLengthMismatch {
            expected: PAYLOAD_LEN,
            actual,
        };
        if actual != PAYLOAD_LEN {
            return Err(mismatch);
        }

        let (full_box_field, rest) = split_field_mut::<4>(buffer, mismatch)?;
        *full_box_field = FullBoxFields::new(0, FullBoxFlags::ZERO).to_bytes();

        let (track_id, rest) = split_field_mut::<4>(rest, mismatch)?;
        *track_id = self.track_id.to_be_bytes();
        let (description_index, rest) = split_field_mut::<4>(rest, mismatch)?;
        *description_index = self.default_sample_description_index.to_be_bytes();
        let (duration, rest) = split_field_mut::<4>(rest, mismatch)?;
        *duration = self.default_sample_duration.to_be_bytes();
        let (size, rest) = split_field_mut::<4>(rest, mismatch)?;
        *size = self.default_sample_size.to_be_bytes();
        let (flags, _rest) = split_field_mut::<4>(rest, mismatch)?;
        *flags = self.default_sample_flags.to_be_bytes();

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, DecodeError};

    use super::TrackExtendsBox;

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let track_extends = TrackExtendsBox::new(1, 1, 1_024, 0, 0x0001_0000);
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

        assert!(matches!(
            TrackExtendsBox::decode_payload(&payload),
            Err(DecodeError::UnsupportedVersion(1))
        ));
    }

    #[test]
    fn a_payload_shorter_than_the_fields_is_rejected() {
        assert!(matches!(
            TrackExtendsBox::decode_payload(&[0; 23]),
            Err(DecodeError::TruncatedPayload {
                needed: 24,
                available: 23
            })
        ));
    }
}
