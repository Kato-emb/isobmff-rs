//! [`UriBox`], the URI that names the form of the metadata of a track

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, Utf8String,
};

use crate::full_box::{read_version_zero, write_version_zero};

/// [`UriBox`] (`uri `), ISO/IEC 14496-12 §12.3.3
///
/// URI that defines the form of the timed metadata, and of its initialization
/// data, in a [`UriMetaSampleEntry`](crate::UriMetaSampleEntry).
///
/// The box reads version 0 alone. The `flags` are read past and written as
/// zero.
#[doc(alias = "uri", alias = "URIBox")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct UriBox {
    /// URI, formatted by the rules of §7.3.3
    pub the_uri: Utf8String,
}

impl BoxDefinition for UriBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"uri ");
}

impl BoxDecode for UriBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {
            the_uri: Utf8String::from_slice(reader.take_remainder()),
        })
    }
}

impl BoxEncode for UriBox {
    fn payload_len(&self) -> u64 {
        self.the_uri.encoded_len().saturating_add(4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        self.the_uri.encode(writer.take_remainder())?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, Error, Utf8String};

    use super::UriBox;
    use crate::encoded;

    /// `uri ` of the text `urn:example:metadata`
    const BOX: &[u8] = b"\0\0\0\x21uri \0\0\0\0urn:example:metadata\0";

    fn value() -> UriBox {
        UriBox {
            the_uri: Utf8String::from_slice(b"urn:example:metadata"),
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(UriBox::decode(BOX), Ok((value(), b"".as_slice())));
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&value()), BOX);
    }

    #[test]
    fn text_without_its_terminator_reads_as_the_text_with_it() {
        assert_eq!(
            UriBox::decode_payload(b"\0\0\0\0urn:example:metadata"),
            Ok(value())
        );
    }

    #[test]
    fn a_version_other_than_0_is_refused() {
        assert_eq!(
            UriBox::decode_payload(b"\x01\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
