//! [`SamplePackingInformationBox`], the samples a sample-packed track preserves

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::full_box::{read_version_zero, write_version_zero};

/// [`SamplePackingInformationBox`] (`spki`), ISO/IEC 14496-12 §13.5.6.2
///
/// Whether a sample-packed track, whose samples may each pack more than one
/// sample of an original track, preserves every sample of the original
/// streams. The box lies in the `SchemeInformationBox` of a restricted
/// scheme of the `scheme_type` `spkt`.
///
/// A reserved `mode` reads as found, and is refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for `mode` when
/// the box is written.
#[doc(alias = "spki")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct SamplePackingInformationBox {
    /// 0 where every sample of the original streams is preserved, 1 where only some are; other values are reserved
    pub mode: u8,
}

impl BoxDefinition for SamplePackingInformationBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"spki");
}

impl BoxDecode for SamplePackingInformationBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;
        let [mode] = *reader.read_bytes::<1>()?;

        Ok(Self { mode })
    }
}

impl BoxEncode for SamplePackingInformationBox {
    fn payload_len(&self) -> u64 {
        4 + 1
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.mode > 1 {
            return Err(Error::invalid_value("mode"));
        }
        write_version_zero(writer)?;
        writer.write_bytes(&[self.mode])
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::SamplePackingInformationBox;
    use crate::encoded;

    /// `spki` of a track that preserves only some of the samples
    const SPKI: &[u8] = b"\0\0\0\x0dspki\0\0\0\0\x01";

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            SamplePackingInformationBox::decode(SPKI),
            Ok((SamplePackingInformationBox { mode: 1 }, b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&SamplePackingInformationBox { mode: 1 }), SPKI);
    }

    #[test]
    fn a_payload_ending_inside_the_mode_is_refused() {
        assert_eq!(
            SamplePackingInformationBox::decode_payload(b"\0\0\0\0"),
            Err(Error::truncated_payload(5, 4))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            SamplePackingInformationBox::decode_payload(b"\x01\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }

    #[test]
    fn a_reserved_mode_is_read_as_found() {
        assert_eq!(
            SamplePackingInformationBox::decode_payload(b"\0\0\0\0\x02"),
            Ok(SamplePackingInformationBox { mode: 2 })
        );
    }

    #[test]
    fn a_reserved_mode_is_refused_when_written() {
        assert_eq!(
            SamplePackingInformationBox { mode: 2 }
                .encode(&mut [0; 13])
                .map(|_| ()),
            Err(Error::invalid_value("mode"))
        );
    }
}
