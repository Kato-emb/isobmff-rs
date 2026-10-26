//! [`MimeBox`], the MIME type of the documents of an XML track

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, Utf8String,
};

use crate::full_box::{read_version_zero, write_version_zero};

/// [`MimeBox`] (`mime`), ISO/IEC 14496-12 §12.3.3
///
/// MIME type each XML document of the track has when delivered on its own,
/// possibly with sub-parameters. Two documents of different MIME types are
/// described by different sample entries.
///
/// The box reads version 0 alone. The `flags` are read past and written as
/// zero.
#[doc(alias = "mime", alias = "MIMEBox")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct MimeBox {
    /// MIME type, sub-parameters included, of each XML document of the track when delivered on its own
    pub content_type: Utf8String,
}

impl BoxDefinition for MimeBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mime");
}

impl BoxDecode for MimeBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {
            content_type: Utf8String::from_slice(reader.take_remainder()),
        })
    }
}

impl BoxEncode for MimeBox {
    fn payload_len(&self) -> u64 {
        self.content_type.encoded_len().saturating_add(4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        self.content_type.encode(writer.take_remainder())?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, Error, Utf8String};

    use super::MimeBox;
    use crate::encoded;

    /// `mime` of the text `application/ttml+xml`
    const BOX: &[u8] = b"\0\0\0\x21mime\0\0\0\0application/ttml+xml\0";

    fn value() -> MimeBox {
        MimeBox {
            content_type: Utf8String::from_slice(b"application/ttml+xml"),
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(MimeBox::decode(BOX), Ok((value(), b"".as_slice())));
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&value()), BOX);
    }

    #[test]
    fn text_without_its_terminator_reads_as_the_text_with_it() {
        assert_eq!(
            MimeBox::decode_payload(b"\0\0\0\0application/ttml+xml"),
            Ok(value())
        );
    }

    #[test]
    fn a_version_other_than_0_is_refused() {
        assert_eq!(
            MimeBox::decode_payload(b"\x01\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
