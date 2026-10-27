//! [`SchemeTypeBox`], the scheme a protection or a restriction follows

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FourCc,
    FullBoxFlags, FullBoxHeader, Utf8String,
};

use crate::full_box::read_version_zero;

/// Flag of `flags` that turns `scheme_uri` on
const SCHEME_URI_PRESENT: u32 = 0x01;

/// [`SchemeTypeBox`] (`schm`), ISO/IEC 14496-12 §13.4.6
///
/// Identifies the protection or restriction scheme, whose information a
/// `SchemeInformationBox` holds.
///
/// `scheme_uri` is [`Some`] where the flag `0x000001` is set: the box is read
/// so wherever the flag is set, and written with the flag set where
/// `scheme_uri` is [`Some`]. The other flags are read past and written as
/// zero.
#[doc(alias = "schm")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SchemeTypeBox {
    /// Code of the scheme, normally a four character code such as `cenc`
    pub scheme_type: FourCc,
    /// Version of the scheme used to create the content
    pub scheme_version: u32,
    /// Absolute URI of a web page for a user who does not have the scheme installed
    pub scheme_uri: Option<Utf8String>,
}

impl Default for SchemeTypeBox {
    fn default() -> Self {
        Self {
            scheme_type: FourCc::new([0; 4]),
            scheme_version: 0,
            scheme_uri: None,
        }
    }
}

impl BoxDefinition for SchemeTypeBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"schm");
}

impl BoxDecode for SchemeTypeBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let flags = read_version_zero(reader)?;
        let scheme_type = FourCc::new(*reader.read_bytes::<4>()?);
        let scheme_version = reader.read_u32()?;

        Ok(Self {
            scheme_type,
            scheme_version,
            scheme_uri: (flags.bits() & SCHEME_URI_PRESENT != 0)
                .then(|| Utf8String::from_slice(reader.take_remainder())),
        })
    }
}

impl BoxEncode for SchemeTypeBox {
    fn payload_len(&self) -> u64 {
        scheme_payload_len(self.scheme_uri.as_ref())
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        encode_scheme(
            writer,
            self.scheme_type,
            self.scheme_version,
            self.scheme_uri.as_ref(),
        )
    }
}

/// Returns the length of the payload of a box of the syntax of [`SchemeTypeBox`] that holds `scheme_uri`
pub(crate) fn scheme_payload_len(scheme_uri: Option<&Utf8String>) -> u64 {
    scheme_uri
        .map_or(0, Utf8String::encoded_len)
        .saturating_add(4 + 4 + 4)
}

/// Writes the payload of a box of the syntax of [`SchemeTypeBox`], with the flag `0x000001` set where there is a `scheme_uri`
///
/// # Errors
///
/// * [`InvalidUtf8String`](isobmff_core::ErrorKind::InvalidUtf8String):
///   `scheme_uri` is not UTF-8.
pub(crate) fn encode_scheme(
    writer: &mut FieldWriter<'_>,
    scheme_type: FourCc,
    scheme_version: u32,
    scheme_uri: Option<&Utf8String>,
) -> Result<(), Error> {
    let flags = if scheme_uri.is_some() {
        FullBoxFlags::new(SCHEME_URI_PRESENT).unwrap_or(FullBoxFlags::ZERO)
    } else {
        FullBoxFlags::ZERO
    };
    writer.write_bytes(&FullBoxHeader::new(0, flags).to_bytes())?;
    writer.write_bytes(scheme_type.as_bytes())?;
    writer.write_u32(scheme_version)?;
    if let Some(scheme_uri) = scheme_uri {
        scheme_uri.encode(writer.take_remainder())?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use isobmff_core::{BoxDecode, Error, FourCc, Utf8String};

    use super::SchemeTypeBox;
    use crate::encoded;

    /// `schm` of the scheme `cenc` version 1.0, of no URI
    const SCHM: &[u8] = b"\0\0\0\x14schm\0\0\0\0cenc\0\x01\0\0";

    /// `schm` of the scheme `cenc` version 1.0, of the URI `https://a.example/`
    const SCHM_URI: &[u8] = b"\0\0\0\x27schm\0\0\0\x01cenc\0\x01\0\0https://a.example/\0";

    fn scheme_type(scheme_uri: Option<&str>) -> SchemeTypeBox {
        SchemeTypeBox {
            scheme_type: FourCc::new(*b"cenc"),
            scheme_version: 0x0001_0000,
            scheme_uri: scheme_uri.map(|uri| Utf8String::new(String::from(uri)).unwrap()),
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            SchemeTypeBox::decode(SCHM),
            Ok((scheme_type(None), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&scheme_type(None)), SCHM);
    }

    #[test]
    fn reads_the_uri_where_the_flag_is_set() {
        assert_eq!(
            SchemeTypeBox::decode(SCHM_URI),
            Ok((scheme_type(Some("https://a.example/")), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_flag_and_the_uri_where_there_is_one() {
        assert_eq!(encoded(&scheme_type(Some("https://a.example/"))), SCHM_URI);
    }

    #[test]
    fn bytes_after_the_version_are_not_a_uri_where_the_flag_is_clear() {
        assert_eq!(
            SchemeTypeBox::decode_payload(b"\0\0\0\x02cenc\0\x01\0\0https://a.example/\0"),
            Ok(scheme_type(None))
        );
    }

    #[test]
    fn a_payload_ending_inside_a_field_is_refused() {
        assert_eq!(
            SchemeTypeBox::decode_payload(b"\0\0\0\0cenc\0\x01"),
            Err(Error::truncated_payload(12, 10))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            SchemeTypeBox::decode_payload(b"\x01\0\0\0cenc\0\x01\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
