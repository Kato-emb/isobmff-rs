//! [`MediaHeaderBox`] (`mdhd`), ISO/IEC 14496-12 §8.4.2

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, DecodeError, EncodeError, FullBoxFields,
    FullBoxFlags,
};

use crate::field::{check_payload_len, split_field, split_field_mut, split_time, write_time};

/// Length of the payload when version 0 carries the times in 32 bits
const PAYLOAD_LEN_VERSION_0: u64 = 24;

/// Length of the payload when version 1 carries the times in 64 bits
const PAYLOAD_LEN_VERSION_1: u64 = 36;

/// Widest value the 15-bit `language` field carries
const LANGUAGE_MAXIMUM: u16 = 0x7FFF;

/// Box that holds the declarations the media of one track applies
///
/// [`MediaHeaderBox`] (`mdhd`), ISO/IEC 14496-12 §8.4.2. The `timescale` here is
/// the track's own, which every sample time in the track is counted in — it is
/// not the movie's, so a reader converting between the two goes through both.
///
/// The version is not held: it selects how wide the times are written, so
/// [`encode_payload`](BoxEncode::encode_payload) picks the narrower one whenever
/// the times fit in 32 bits.
#[doc(alias = "mdhd")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct MediaHeaderBox {
    creation_time: u64,
    modification_time: u64,
    timescale: u32,
    duration: u64,
    language: u16,
    pre_defined: u16,
}

impl MediaHeaderBox {
    /// Creates the box from the declarations of one track's media
    ///
    /// `language` is an ISO-639-2/T code packed as three five-bit letters, so
    /// it occupies the low 15 bits. Returns `None` for a wider value, which the
    /// field cannot carry.
    #[must_use]
    pub const fn new(
        creation_time: u64,
        modification_time: u64,
        timescale: u32,
        duration: u64,
        language: u16,
    ) -> Option<Self> {
        if language > LANGUAGE_MAXIMUM {
            return None;
        }

        Some(Self {
            creation_time,
            modification_time,
            timescale,
            duration,
            language,
            pre_defined: 0,
        })
    }

    /// Returns the time the media was created, in seconds since 1904-01-01
    #[must_use]
    pub const fn creation_time(&self) -> u64 {
        self.creation_time
    }

    /// Returns the time the media was last modified, on the same scale
    #[must_use]
    pub const fn modification_time(&self) -> u64 {
        self.modification_time
    }

    /// Returns how many units of the track's time scale pass in one second
    #[must_use]
    pub const fn timescale(&self) -> u32 {
        self.timescale
    }

    /// Returns the length of the track, in the track's own time scale
    #[must_use]
    pub const fn duration(&self) -> u64 {
        self.duration
    }

    /// Returns the ISO-639-2/T language code, packed as three five-bit letters
    #[must_use]
    pub const fn language(&self) -> u16 {
        self.language
    }

    /// Returns the field the spec reserves for a later definition
    #[must_use]
    pub const fn pre_defined(&self) -> u16 {
        self.pre_defined
    }

    /// Returns the version whose field widths carry the times of this box
    const fn version(&self) -> u8 {
        let widest = u32::MAX as u64;
        let fits_in_32_bits = self.creation_time <= widest
            && self.modification_time <= widest
            && self.duration <= widest;

        if fits_in_32_bits { 0 } else { 1 }
    }
}

impl BoxDefinition for MediaHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mdhd");
}

impl BoxDecode for MediaHeaderBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](DecodeError::UnsupportedVersion): the box
    ///   declares a version other than 0 or 1.
    /// * [`TruncatedPayload`](DecodeError::TruncatedPayload): the payload is
    ///   shorter than the version it declares requires.
    /// * [`TrailingBytes`](DecodeError::TrailingBytes): the payload is longer.
    fn decode_payload(payload: &[u8]) -> Result<Self, DecodeError> {
        let available = u64::try_from(payload.len()).unwrap_or(u64::MAX);
        let (full_box_field, rest) = split_field::<4>(payload, 4, available)?;
        let version = FullBoxFields::from_bytes(full_box_field).version();

        let needed = match version {
            0 => PAYLOAD_LEN_VERSION_0,
            1 => PAYLOAD_LEN_VERSION_1,
            unsupported => return Err(DecodeError::UnsupportedVersion(unsupported)),
        };
        check_payload_len(needed, available)?;

        let (creation_time, rest) = split_time(version, rest, needed, available)?;
        let (modification_time, rest) = split_time(version, rest, needed, available)?;
        let (timescale, rest) = split_field::<4>(rest, needed, available)?;
        let (duration, rest) = split_time(version, rest, needed, available)?;
        let (language, rest) = split_field::<2>(rest, needed, available)?;
        let (pre_defined, _rest) = split_field::<2>(rest, needed, available)?;

        Ok(Self {
            creation_time,
            modification_time,
            timescale: u32::from_be_bytes(*timescale),
            duration,
            language: u16::from_be_bytes(*language) & LANGUAGE_MAXIMUM,
            pre_defined: u16::from_be_bytes(*pre_defined),
        })
    }
}

impl BoxEncode for MediaHeaderBox {
    fn payload_len(&self) -> u64 {
        if self.version() == 0 {
            PAYLOAD_LEN_VERSION_0
        } else {
            PAYLOAD_LEN_VERSION_1
        }
    }

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), EncodeError> {
        let expected = self.payload_len();
        let actual = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        let mismatch = EncodeError::BufferLengthMismatch { expected, actual };
        if actual != expected {
            return Err(mismatch);
        }

        let version = self.version();
        let (full_box_field, rest) = split_field_mut::<4>(buffer, mismatch)?;
        *full_box_field = FullBoxFields::new(version, FullBoxFlags::ZERO).to_bytes();

        let rest = write_time(version, self.creation_time, rest, mismatch)?;
        let rest = write_time(version, self.modification_time, rest, mismatch)?;
        let (timescale, rest) = split_field_mut::<4>(rest, mismatch)?;
        *timescale = self.timescale.to_be_bytes();
        let rest = write_time(version, self.duration, rest, mismatch)?;
        let (language, rest) = split_field_mut::<2>(rest, mismatch)?;
        *language = self.language.to_be_bytes();
        let (pre_defined, _rest) = split_field_mut::<2>(rest, mismatch)?;
        *pre_defined = self.pre_defined.to_be_bytes();

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, DecodeError};

    use super::{LANGUAGE_MAXIMUM, MediaHeaderBox};

    /// `und`, the code a writer uses when the language is undetermined
    const UNDETERMINED: u16 = 0x55C4;

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(media_header: &MediaHeaderBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(media_header.payload_len()).unwrap()];
        media_header.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_language_wider_than_the_field_is_refused() {
        assert_eq!(MediaHeaderBox::new(0, 0, 1, 0, LANGUAGE_MAXIMUM + 1), None);
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it_at_either_version() {
        for duration in [u64::from(u32::MAX), u64::from(u32::MAX) + 1] {
            let media_header = MediaHeaderBox::new(1, 2, 90_000, duration, UNDETERMINED).unwrap();

            let payload = encoded_payload(&media_header);

            assert_eq!(
                MediaHeaderBox::decode_payload(&payload).unwrap(),
                media_header
            );
        }
    }

    #[test]
    fn the_pad_bit_above_the_language_is_dropped_on_the_way_in() {
        let media_header = MediaHeaderBox::new(0, 0, 1, 0, UNDETERMINED).unwrap();
        let mut payload = encoded_payload(&media_header);
        let language = payload.get_mut(20..22).unwrap();
        language.copy_from_slice(&(UNDETERMINED | 0x8000).to_be_bytes());

        assert_eq!(
            MediaHeaderBox::decode_payload(&payload).unwrap(),
            media_header
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let mut payload = vec![0; 24];
        *payload.first_mut().unwrap() = 2;

        assert!(matches!(
            MediaHeaderBox::decode_payload(&payload),
            Err(DecodeError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn a_payload_longer_than_its_version_requires_is_rejected() {
        assert!(matches!(
            MediaHeaderBox::decode_payload(&[0; 25]),
            Err(DecodeError::TrailingBytes { remaining: 1 })
        ));
    }
}
