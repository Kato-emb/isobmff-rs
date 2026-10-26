//! [`TextConfigBox`], the text each document of a text track opens with

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, Utf8String,
};

use crate::full_box::{read_version_zero, write_version_zero};

/// [`TextConfigBox`] (`txtC`), ISO/IEC 14496-12 §12.3.3
///
/// Text that each document of the track opens with: a reader prepends it to the
/// contents of each sync sample.
///
/// The box reads version 0 alone. The `flags` are read past and written as
/// zero.
#[doc(alias = "txtC")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct TextConfigBox {
    /// Initial text of each document, prepended to the contents of each sync sample
    pub text_config: Utf8String,
}

impl BoxDefinition for TextConfigBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"txtC");
}

impl BoxDecode for TextConfigBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {
            text_config: Utf8String::from_slice(reader.take_remainder()),
        })
    }
}

impl BoxEncode for TextConfigBox {
    fn payload_len(&self) -> u64 {
        self.text_config.encoded_len().saturating_add(4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        self.text_config.encode(writer.take_remainder())?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, Error, Utf8String};

    use super::TextConfigBox;
    use crate::encoded;

    /// `txtC` of the text `WEBVTT`
    const BOX: &[u8] = b"\0\0\0\x13txtC\0\0\0\0WEBVTT\0";

    fn value() -> TextConfigBox {
        TextConfigBox {
            text_config: Utf8String::from_slice(b"WEBVTT"),
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(TextConfigBox::decode(BOX), Ok((value(), b"".as_slice())));
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&value()), BOX);
    }

    #[test]
    fn text_without_its_terminator_reads_as_the_text_with_it() {
        assert_eq!(
            TextConfigBox::decode_payload(b"\0\0\0\0WEBVTT"),
            Ok(value())
        );
    }

    #[test]
    fn a_version_other_than_0_is_refused() {
        assert_eq!(
            TextConfigBox::decode_payload(b"\x01\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
