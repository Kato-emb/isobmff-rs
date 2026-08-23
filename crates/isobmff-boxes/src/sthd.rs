//! [`SubtitleMediaHeaderBox`] (`sthd`), ISO/IEC 14496-12 §12.6.2

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFields,
    FullBoxFlags,
};

/// Length of the payload, which the version and the flags are the whole of
const PAYLOAD_LEN: u64 = 4;

/// Box that stands as the media header of a subtitle track
///
/// [`SubtitleMediaHeaderBox`] (`sthd`), ISO/IEC 14496-12 §12.6.2. A subtitle
/// track takes this as the media header its `minf` must hold. The box carries
/// no field of its own.
///
/// Neither the version nor the `flags` are held — the spec declares the version
/// zero and the flags all zero for this box.
#[doc(alias = "sthd")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct SubtitleMediaHeaderBox;

impl SubtitleMediaHeaderBox {
    /// Creates the box, which states nothing beyond the kind of media it heads
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl BoxDefinition for SubtitleMediaHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"sthd");
}

impl BoxDecode for SubtitleMediaHeaderBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
    ///   ends inside the version and the flags.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version != 0 {
            return Err(Error::unsupported_version(version));
        }

        Ok(Self)
    }
}

impl BoxEncode for SubtitleMediaHeaderBox {
    fn payload_len(&self) -> u64 {
        PAYLOAD_LEN
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxFields::new(0, FullBoxFlags::ZERO).to_bytes())?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::SubtitleMediaHeaderBox;

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let subtitle = SubtitleMediaHeaderBox::new();
        let mut payload = vec![0xff; 4];

        subtitle.encode_payload(&mut payload).unwrap();

        assert_eq!(payload, b"\0\0\0\0");
        assert_eq!(
            SubtitleMediaHeaderBox::decode_payload(&payload).unwrap(),
            subtitle
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        assert_eq!(
            SubtitleMediaHeaderBox::decode_payload(b"\x01\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }

    #[test]
    fn a_payload_shorter_than_the_version_and_the_flags_is_rejected() {
        assert_eq!(
            SubtitleMediaHeaderBox::decode_payload(&[0; 3]),
            Err(Error::truncated_payload(4, 3))
        );
    }
}
