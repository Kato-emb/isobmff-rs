//! [`CopyrightBox`], a copyright notice of a presentation or a track

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, UtfString,
};

use crate::data_types::LanguageCode;
use crate::full_box::{read_version_zero, write_version_zero};

/// [`CopyrightBox`] (`cprt`), ISO/IEC 14496-12 §8.10.2
///
/// Copyright notice that applies to the whole presentation when the box is in
/// the `UserDataBox` of the `MovieBox`, or to the whole track when it is in the
/// `UserDataBox` of a track. A presentation or a track may hold one in each of
/// several languages.
///
/// The box reads version 0 alone. The `flags` and the bit before `language`
/// are read past and written as zero.
#[doc(alias = "cprt")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct CopyrightBox {
    /// Language of `notice`
    pub language: LanguageCode,
    /// Copyright notice, in UTF-8 or UTF-16
    pub notice: UtfString,
}

impl BoxDefinition for CopyrightBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"cprt");
}

impl BoxDecode for CopyrightBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {
            language: LanguageCode::from_bits(reader.read_u16()?),
            notice: UtfString::from_slice(reader.take_remainder()),
        })
    }
}

impl BoxEncode for CopyrightBox {
    fn payload_len(&self) -> u64 {
        self.notice.encoded_len().saturating_add(4 + 2)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        writer.write_u16(self.language.bits())?;
        self.notice.encode(writer.take_remainder())?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use isobmff_core::{BoxDecode, Error, UtfString};

    use super::CopyrightBox;
    use crate::data_types::LanguageCode;
    use crate::encoded;

    /// `cprt` of the language `eng` and the UTF-8 notice `(c) 2026`
    const CPRT: &[u8] = b"\0\0\0\x17cprt\0\0\0\0\x15\xc7(c) 2026\0";

    /// `cprt` of the language `und` and the big-endian UTF-16 notice `(c)`
    const CPRT_UTF16: &[u8] = b"\0\0\0\x18cprt\0\0\0\0\x55\xc4\xfe\xff\0(\0c\0)\0\0";

    fn copyright() -> CopyrightBox {
        CopyrightBox {
            language: LanguageCode::new(*b"eng").unwrap(),
            notice: UtfString::new(String::from("(c) 2026")).unwrap(),
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            CopyrightBox::decode(CPRT),
            Ok((copyright(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&copyright()), CPRT);
    }

    #[test]
    fn a_notice_in_utf_16_is_written_back_in_utf_16() {
        let utf16 = CopyrightBox {
            language: LanguageCode::UND,
            notice: UtfString::from_slice(b"\xfe\xff\0(\0c\0)"),
        };

        assert_eq!(
            (CopyrightBox::decode(CPRT_UTF16), encoded(&utf16)),
            (Ok((utf16.clone(), b"".as_slice())), CPRT_UTF16.to_vec())
        );
    }

    #[test]
    fn a_payload_ending_inside_the_language_is_refused() {
        assert_eq!(
            CopyrightBox::decode_payload(b"\0\0\0\0\x15"),
            Err(Error::truncated_payload(6, 5))
        );
    }

    #[test]
    fn a_version_other_than_0_is_refused() {
        assert_eq!(
            CopyrightBox::decode_payload(b"\x01\0\0\0\x15\xc7\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
