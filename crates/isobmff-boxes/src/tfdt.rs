//! [`TrackFragmentBaseMediaDecodeTimeBox`] (`tfdt`), ISO/IEC 14496-12 §8.8.12

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWidth, FieldWriter,
    FullBoxFields, FullBoxFlags,
};

/// Length of the payload when version 0 carries the time in 32 bits
const PAYLOAD_LEN_VERSION_0: u64 = 8;

/// Length of the payload when version 1 carries the time in 64 bits
const PAYLOAD_LEN_VERSION_1: u64 = 12;

/// Box that states the decode time the samples of one track fragment start at
///
/// [`TrackFragmentBaseMediaDecodeTimeBox`] (`tfdt`), ISO/IEC 14496-12 §8.8.12.
/// The `base_media_decode_time` is the decode time of the first sample of the
/// fragment, measured on the media timeline, so a reader seeking into a file
/// reaches it without summing the durations of every sample before it.
///
/// The version is not held: it selects how wide the time is written, so
/// [`encode_payload`](BoxEncode::encode_payload) picks the narrower one whenever
/// the time fits in 32 bits. The `flags` are not held either — the spec declares
/// them zero for this box.
#[doc(alias = "tfdt")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TrackFragmentBaseMediaDecodeTimeBox {
    base_media_decode_time: u64,
}

impl TrackFragmentBaseMediaDecodeTimeBox {
    /// Creates the box from the decode time its fragment starts at
    #[must_use]
    pub const fn new(base_media_decode_time: u64) -> Self {
        Self {
            base_media_decode_time,
        }
    }

    /// Returns the decode time of the first sample, in the media time scale
    #[must_use]
    pub const fn base_media_decode_time(&self) -> u64 {
        self.base_media_decode_time
    }

    /// Returns the version whose field width carries the time of this box
    const fn version(&self) -> u8 {
        if self.base_media_decode_time <= u32::MAX as u64 {
            0
        } else {
            1
        }
    }

    /// Returns the width the given version carries the time of this box at
    const fn field_width(version: u8) -> FieldWidth {
        match version {
            0 => FieldWidth::Compact,
            _ => FieldWidth::Extended,
        }
    }
}

impl BoxDefinition for TrackFragmentBaseMediaDecodeTimeBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tfdt");
}

impl BoxDecode for TrackFragmentBaseMediaDecodeTimeBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0 or 1.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload) or
    ///   [`TrailingPayload`](isobmff_core::ErrorKind::TrailingPayload): the payload ends inside a
    ///   field, or holds bytes past the fields of the box.
    fn decode_payload(payload: &[u8]) -> Result<Self, Error> {
        let mut reader = FieldReader::new(payload);
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version > 1 {
            return Err(Error::unsupported_version(version));
        }

        let base_media_decode_time = reader.read_unsigned(Self::field_width(version))?;
        reader.finish()?;

        Ok(Self {
            base_media_decode_time,
        })
    }
}

impl BoxEncode for TrackFragmentBaseMediaDecodeTimeBox {
    fn payload_len(&self) -> u64 {
        if self.version() == 0 {
            PAYLOAD_LEN_VERSION_0
        } else {
            PAYLOAD_LEN_VERSION_1
        }
    }

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), Error> {
        let expected = self.payload_len();
        let actual = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        if actual != expected {
            return Err(Error::buffer_length_mismatch(expected, actual));
        }

        let version = self.version();
        let mut writer = FieldWriter::new(buffer);

        writer.write_bytes(&FullBoxFields::new(version, FullBoxFlags::ZERO).to_bytes())?;
        writer.write_unsigned(Self::field_width(version), self.base_media_decode_time)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::TrackFragmentBaseMediaDecodeTimeBox;

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(decode_time: &TrackFragmentBaseMediaDecodeTimeBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(decode_time.payload_len()).unwrap()];
        decode_time.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_time_within_32_bits_is_written_at_version_0() {
        let payload = encoded_payload(&TrackFragmentBaseMediaDecodeTimeBox::new(u64::from(
            u32::MAX,
        )));

        assert_eq!(payload, b"\0\0\0\0\xff\xff\xff\xff");
    }

    #[test]
    fn a_time_past_32_bits_moves_to_version_1() {
        let payload = encoded_payload(&TrackFragmentBaseMediaDecodeTimeBox::new(
            u64::from(u32::MAX) + 1,
        ));

        assert_eq!(payload, b"\x01\0\0\0\0\0\0\x01\0\0\0\0");
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it_at_either_version() {
        for time in [u64::from(u32::MAX), u64::from(u32::MAX) + 1] {
            let decode_time = TrackFragmentBaseMediaDecodeTimeBox::new(time);

            let payload = encoded_payload(&decode_time);

            assert_eq!(
                TrackFragmentBaseMediaDecodeTimeBox::decode_payload(&payload).unwrap(),
                decode_time
            );
        }
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let mut payload = vec![0; 8];
        *payload.first_mut().unwrap() = 2;

        assert_eq!(
            TrackFragmentBaseMediaDecodeTimeBox::decode_payload(&payload),
            Err(Error::unsupported_version(2))
        );
    }

    #[test]
    fn a_payload_shorter_than_its_version_requires_is_rejected() {
        let payload = encoded_payload(&TrackFragmentBaseMediaDecodeTimeBox::new(
            u64::from(u32::MAX) + 1,
        ));

        assert_eq!(
            TrackFragmentBaseMediaDecodeTimeBox::decode_payload(payload.get(..11).unwrap()),
            Err(Error::truncated_payload(12, 11))
        );
    }
}
