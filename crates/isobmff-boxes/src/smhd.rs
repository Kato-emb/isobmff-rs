//! [`SoundMediaHeaderBox`] (`smhd`), ISO/IEC 14496-12 §12.2.2

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFields,
    FullBoxFlags, I8F8,
};

/// Length of the payload, which has no version-dependent field
const PAYLOAD_LEN: u64 = 8;

/// Box that states where the audio of a track sits in a stereo space
///
/// [`SoundMediaHeaderBox`] (`smhd`), ISO/IEC 14496-12 §12.2.2. An audio track
/// takes this as the media header its `minf` must hold. The `balance` places a
/// mono track between the two channels: 0 is centre, -1.0 is full left, and 1.0
/// is full right.
///
/// The `reserved` field is not held — the spec declares it zero — and neither
/// is the version or the `flags`, which the spec declares zero as well.
#[doc(alias = "smhd")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SoundMediaHeaderBox {
    balance: I8F8,
}

impl SoundMediaHeaderBox {
    /// Creates the box from the place its track takes between the channels
    #[must_use]
    pub const fn new(balance: I8F8) -> Self {
        Self { balance }
    }

    /// Returns where a mono track sits between the two channels
    #[must_use]
    pub const fn balance(&self) -> I8F8 {
        self.balance
    }
}

impl BoxDefinition for SoundMediaHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"smhd");
}

impl BoxDecode for SoundMediaHeaderBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
    ///   ends inside a field of the box.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version != 0 {
            return Err(Error::unsupported_version(version));
        }

        let balance = I8F8::from_raw(reader.read_i16()?);
        let _reserved = reader.read_bytes::<2>()?;

        Ok(Self { balance })
    }
}

impl BoxEncode for SoundMediaHeaderBox {
    fn payload_len(&self) -> u64 {
        PAYLOAD_LEN
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxFields::new(0, FullBoxFlags::ZERO).to_bytes())?;
        writer.write_i16(self.balance.raw())?;
        writer.write_bytes(&[0; 2])?;

        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error, I8F8};

    use super::SoundMediaHeaderBox;

    /// Media header of an audio track sitting at the centre of the stereo space
    pub(crate) fn sound_media_header() -> SoundMediaHeaderBox {
        SoundMediaHeaderBox::new(I8F8::ZERO)
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(sound_media_header: &SoundMediaHeaderBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(sound_media_header.payload_len()).unwrap()];
        sound_media_header.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let sound_media_header = SoundMediaHeaderBox::new(I8F8::from_raw(-256));

        let payload = encoded_payload(&sound_media_header);

        assert_eq!(payload, b"\0\0\0\0\xff\0\0\0");
        assert_eq!(
            SoundMediaHeaderBox::decode_payload(&payload).unwrap(),
            sound_media_header
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let mut payload = vec![0; 8];
        *payload.first_mut().unwrap() = 1;

        assert_eq!(
            SoundMediaHeaderBox::decode_payload(&payload),
            Err(Error::unsupported_version(1))
        );
    }

    #[test]
    fn a_payload_shorter_than_the_fields_is_rejected() {
        assert_eq!(
            SoundMediaHeaderBox::decode_payload(&[0; 7]),
            Err(Error::truncated_payload(8, 7))
        );
    }
}
