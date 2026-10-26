//! [`HandlerProperty`], the media handler of an item

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FourCc,
};

use crate::full_box::{read_version_zero, write_version_zero};

/// [`HandlerProperty`] (`hdlp`), ISO/IEC 14496-12 §8.11.16
///
/// A descriptive item property that maps an item of a `MetaBox` to a media
/// handler.
///
/// [`Default`] is the property of `handler_type` four zero bytes.
#[doc(alias = "hdlp")]
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct HandlerProperty {
    /// Media handler type of the item, such as `pict`, or `null` for none in particular
    pub handler_type: FourCc,
}

impl Default for HandlerProperty {
    fn default() -> Self {
        Self {
            handler_type: FourCc::new([0; 4]),
        }
    }
}

impl BoxDefinition for HandlerProperty {
    const BOX_TYPE: BoxType = BoxType::compact(*b"hdlp");
}

impl BoxDecode for HandlerProperty {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {
            handler_type: FourCc::new(*reader.read_bytes::<4>()?),
        })
    }
}

impl BoxEncode for HandlerProperty {
    fn payload_len(&self) -> u64 {
        4 + 4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        writer.write_bytes(self.handler_type.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, Error, FourCc};

    use super::HandlerProperty;
    use crate::encoded;

    /// `hdlp` of the handler type `pict`
    const HDLP: &[u8] = b"\0\0\0\x10hdlp\0\0\0\0pict";

    fn handler() -> HandlerProperty {
        HandlerProperty {
            handler_type: FourCc::new(*b"pict"),
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            HandlerProperty::decode(HDLP),
            Ok((handler(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&handler()), HDLP);
    }

    #[test]
    fn a_payload_ending_inside_the_handler_type_is_refused() {
        assert_eq!(
            HandlerProperty::decode_payload(b"\0\0\0\0pi"),
            Err(Error::truncated_payload(8, 6))
        );
    }

    #[test]
    fn a_version_the_property_does_not_read_is_refused() {
        assert_eq!(
            HandlerProperty::decode_payload(b"\x01\0\0\0pict"),
            Err(Error::unsupported_version(1))
        );
    }
}
