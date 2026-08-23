//! [`HintMediaHeaderBox`] (`hmhd`), ISO/IEC 14496-12 §12.4.2

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFields,
    FullBoxFlags,
};

/// Length of the payload, which has no version-dependent field
const PAYLOAD_LEN: u64 = 20;

/// Box that states the sizes and rates the packets of a hint track reach
///
/// [`HintMediaHeaderBox`] (`hmhd`), ISO/IEC 14496-12 §12.4.2. A hint track
/// takes this as the media header its `minf` must hold. The four fields measure
/// the protocol data units the track hints at: the largest and the average of
/// them in bytes, and the largest and the average rate in bits per second, the
/// largest taken over any one-second window and the average over the whole
/// presentation.
///
/// The `reserved` field is not held — the spec declares it zero — and neither
/// is the version or the `flags`, which the spec declares zero as well.
#[doc(alias = "hmhd")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct HintMediaHeaderBox {
    max_pdu_size: u16,
    avg_pdu_size: u16,
    max_bitrate: u32,
    avg_bitrate: u32,
}

impl HintMediaHeaderBox {
    /// Creates the box from the sizes and the rates its packets reach
    #[must_use]
    pub const fn new(
        max_pdu_size: u16,
        avg_pdu_size: u16,
        max_bitrate: u32,
        avg_bitrate: u32,
    ) -> Self {
        Self {
            max_pdu_size,
            avg_pdu_size,
            max_bitrate,
            avg_bitrate,
        }
    }

    /// Returns the size in bytes of the largest packet of the track
    #[must_use]
    pub const fn max_pdu_size(&self) -> u16 {
        self.max_pdu_size
    }

    /// Returns the size a packet of the track takes on average over the presentation
    #[must_use]
    pub const fn avg_pdu_size(&self) -> u16 {
        self.avg_pdu_size
    }

    /// Returns the highest rate the track reaches over any one second
    #[must_use]
    pub const fn max_bitrate(&self) -> u32 {
        self.max_bitrate
    }

    /// Returns the rate the track holds on average over the presentation
    #[must_use]
    pub const fn avg_bitrate(&self) -> u32 {
        self.avg_bitrate
    }
}

impl BoxDefinition for HintMediaHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"hmhd");
}

impl BoxDecode for HintMediaHeaderBox {
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

        let max_pdu_size = reader.read_u16()?;
        let avg_pdu_size = reader.read_u16()?;
        let max_bitrate = reader.read_u32()?;
        let avg_bitrate = reader.read_u32()?;
        let _reserved = reader.read_bytes::<4>()?;

        Ok(Self {
            max_pdu_size,
            avg_pdu_size,
            max_bitrate,
            avg_bitrate,
        })
    }
}

impl BoxEncode for HintMediaHeaderBox {
    fn payload_len(&self) -> u64 {
        PAYLOAD_LEN
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxFields::new(0, FullBoxFlags::ZERO).to_bytes())?;
        writer.write_u16(self.max_pdu_size)?;
        writer.write_u16(self.avg_pdu_size)?;
        writer.write_u32(self.max_bitrate)?;
        writer.write_u32(self.avg_bitrate)?;
        writer.write_bytes(&[0; 4])?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::HintMediaHeaderBox;

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let hint_media_header = HintMediaHeaderBox::new(1500, 1200, 800_000, 600_000);
        let mut payload = vec![0; 20];

        hint_media_header.encode_payload(&mut payload).unwrap();

        assert_eq!(
            payload,
            b"\0\0\0\0\x05\xdc\x04\xb0\0\x0c\x35\0\0\x09\x27\xc0\0\0\0\0"
        );
        assert_eq!(
            HintMediaHeaderBox::decode_payload(&payload).unwrap(),
            hint_media_header
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let mut payload = vec![0; 20];
        *payload.first_mut().unwrap() = 1;

        assert_eq!(
            HintMediaHeaderBox::decode_payload(&payload),
            Err(Error::unsupported_version(1))
        );
    }

    #[test]
    fn a_payload_shorter_than_the_fields_is_rejected() {
        assert_eq!(
            HintMediaHeaderBox::decode_payload(&[0; 19]),
            Err(Error::truncated_payload(20, 19))
        );
    }
}
