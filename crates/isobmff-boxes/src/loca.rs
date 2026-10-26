//! [`MetadataLocaleBox`], the locale of the values of a metadata key

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, Utf8String,
};

/// [`MetadataLocaleBox`] (`loca`), ISO/IEC 14496-12 §12.8.4.5
///
/// The locale the values of a metadata key are for. Values of a key without
/// this box are for every locale.
#[doc(alias = "loca")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct MetadataLocaleBox {
    /// IETF BCP 47 language tag, such as `en-US`
    pub locale_string: Utf8String,
}

impl BoxDefinition for MetadataLocaleBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"loca");
}

impl BoxDecode for MetadataLocaleBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            locale_string: Utf8String::from_slice(reader.take_remainder()),
        })
    }
}

impl BoxEncode for MetadataLocaleBox {
    fn payload_len(&self) -> u64 {
        self.locale_string.encoded_len()
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        self.locale_string.encode(writer.take_remainder())?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use isobmff_core::{BoxDecode, Utf8String};

    use super::MetadataLocaleBox;
    use crate::encoded;

    /// `loca` of `fr-FR`
    const LOCA: &[u8] = b"\0\0\0\x0elocafr-FR\0";

    fn locale() -> MetadataLocaleBox {
        MetadataLocaleBox {
            locale_string: Utf8String::new(String::from("fr-FR")).unwrap(),
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            MetadataLocaleBox::decode(LOCA),
            Ok((locale(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&locale()), LOCA);
    }
}
