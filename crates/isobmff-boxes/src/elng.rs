//! [`ExtendedLanguageBox`], the language of a medium as a BCP 47 tag

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, Utf8String,
};

use crate::full_box::{read_version_zero, write_version_zero};

/// [`ExtendedLanguageBox`] (`elng`), ISO/IEC 14496-12 §8.4.6
///
/// Language of the media as an IETF BCP 47 language tag, which overrides the
/// `language` of the `MediaHeaderBox` where the two disagree.
#[doc(alias = "elng")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct ExtendedLanguageBox {
    /// IETF BCP 47 language tag, such as `en-US`
    pub extended_language: Utf8String,
}

impl BoxDefinition for ExtendedLanguageBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"elng");
}

impl BoxDecode for ExtendedLanguageBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {
            extended_language: Utf8String::from_slice(reader.take_remainder()),
        })
    }
}

impl BoxEncode for ExtendedLanguageBox {
    fn payload_len(&self) -> u64 {
        self.extended_language.encoded_len().saturating_add(4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        self.extended_language.encode(writer.take_remainder())?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use isobmff_core::{BoxDecode, Error, Utf8String};

    use super::ExtendedLanguageBox;
    use crate::encoded;

    /// `elng` of the tag `en-US`
    const ELNG: &[u8] = b"\0\0\0\x12elng\0\0\0\0en-US\0";

    fn extended_language() -> ExtendedLanguageBox {
        ExtendedLanguageBox {
            extended_language: Utf8String::new(String::from("en-US")).unwrap(),
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            ExtendedLanguageBox::decode(ELNG),
            Ok((extended_language(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&extended_language()), ELNG);
    }

    #[test]
    fn a_payload_ending_inside_the_version_and_flags_is_refused() {
        assert_eq!(
            ExtendedLanguageBox::decode_payload(b"\0\0"),
            Err(Error::truncated_payload(4, 2))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            ExtendedLanguageBox::decode_payload(b"\x01\0\0\0en\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
