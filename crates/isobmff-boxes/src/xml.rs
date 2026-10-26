//! [`XmlBox`] and [`BinaryXmlBox`], the variants that hold the XML of a meta box

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, BoxVariants, Error, FieldReader, FieldWriter,
    RawBox, UtfString,
};

use crate::full_box::{read_version_zero, write_version_zero};

/// [`XmlBox`] (`xml `), ISO/IEC 14496-12 §8.11.2
///
/// XML stored directly in a `MetaBox`, as text. §8.11.2 deprecates the box in
/// new specifications in favour of an item.
///
/// The text is read whether or not the null terminator that the 2026 edition
/// requires is there, and written with it.
#[doc(alias = "xml", alias = "XMLBox")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct XmlBox {
    /// XML data
    pub xml: UtfString,
}

impl BoxDefinition for XmlBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"xml ");
}

impl BoxDecode for XmlBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {
            xml: UtfString::from_slice(reader.take_remainder()),
        })
    }
}

impl BoxEncode for XmlBox {
    fn payload_len(&self) -> u64 {
        self.xml.encoded_len().saturating_add(4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        self.xml.encode(writer.take_remainder())?;

        Ok(())
    }
}

/// [`BinaryXmlBox`] (`bxml`), ISO/IEC 14496-12 §8.11.2
///
/// XML stored directly in a `MetaBox`, in the one binarization the handler of
/// the `MetaBox` defines.
#[doc(alias = "bxml", alias = "BinaryXMLBox")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct BinaryXmlBox {
    /// Encoded XML data
    pub data: Vec<u8>,
}

impl BoxDefinition for BinaryXmlBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"bxml");
}

impl BoxDecode for BinaryXmlBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {
            data: reader.take_remainder().to_vec(),
        })
    }
}

impl BoxEncode for BinaryXmlBox {
    fn payload_len(&self) -> u64 {
        u64::try_from(self.data.len())
            .unwrap_or(u64::MAX)
            .saturating_add(4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        writer.write_slice(&self.data)
    }
}

/// XML of a `MetaBox`, stored by one of the variants of ISO/IEC 14496-12 §8.11.2
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum XmlVariant {
    /// XML as text
    Xml(XmlBox),
    /// XML in a binary form
    BinaryXml(BinaryXmlBox),
}

impl BoxVariants for XmlVariant {
    const VARIANTS: &'static [BoxType] = &[XmlBox::BOX_TYPE, BinaryXmlBox::BOX_TYPE];

    fn decode_variant(child: RawBox<'_>) -> Result<Self, Error> {
        if child.header().box_type() == XmlBox::BOX_TYPE {
            XmlBox::decode_payload(child.payload()).map(Self::Xml)
        } else {
            BinaryXmlBox::decode_payload(child.payload()).map(Self::BinaryXml)
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec;

    use isobmff_core::{BoxDecode, Error, UtfString};

    use super::{BinaryXmlBox, XmlBox};
    use crate::encoded;

    /// `xml ` holding `<a/>`
    const XML: &[u8] = b"\0\0\0\x11xml \0\0\0\0<a/>\0";

    /// `bxml` holding three bytes of binary XML
    const BXML: &[u8] = b"\0\0\0\x0fbxml\0\0\0\0\x01\x02\x03";

    fn xml() -> XmlBox {
        XmlBox {
            xml: UtfString::new(String::from("<a/>")).unwrap(),
        }
    }

    fn binary_xml() -> BinaryXmlBox {
        BinaryXmlBox {
            data: vec![1, 2, 3],
        }
    }

    #[test]
    fn each_variant_reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            (XmlBox::decode(XML), BinaryXmlBox::decode(BXML)),
            (
                Ok((xml(), b"".as_slice())),
                Ok((binary_xml(), b"".as_slice()))
            )
        );
    }

    #[test]
    fn each_variant_writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            (encoded(&xml()), encoded(&binary_xml())),
            (XML.to_vec(), BXML.to_vec())
        );
    }

    #[test]
    fn xml_without_its_null_terminator_reads_and_is_written_with_it() {
        let (read, _) = XmlBox::decode(b"\0\0\0\x10xml \0\0\0\0<a/>").unwrap();

        assert_eq!(encoded(&read), XML);
    }

    #[test]
    fn a_payload_ending_inside_the_full_box_fields_is_refused() {
        assert_eq!(
            (
                XmlBox::decode_payload(b"\0\0"),
                BinaryXmlBox::decode_payload(b"\0\0")
            ),
            (
                Err(Error::truncated_payload(4, 2)),
                Err(Error::truncated_payload(4, 2))
            )
        );
    }

    #[test]
    fn a_version_the_variants_do_not_read_is_refused() {
        assert_eq!(
            (
                XmlBox::decode_payload(b"\x01\0\0\0"),
                BinaryXmlBox::decode_payload(b"\x01\0\0\0")
            ),
            (
                Err(Error::unsupported_version(1)),
                Err(Error::unsupported_version(1))
            )
        );
    }
}
