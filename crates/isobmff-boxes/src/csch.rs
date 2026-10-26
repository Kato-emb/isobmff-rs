//! [`CompatibleSchemeTypeBox`], a further restriction scheme a track conforms to

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FourCc,
    Utf8String,
};

use crate::schm::{SchemeTypeBox, encode_scheme, scheme_payload_len};

/// [`CompatibleSchemeTypeBox`] (`csch`), ISO/IEC 14496-12 §13.5.5
///
/// A scheme the track conforms to besides the one the `SchemeTypeBox` of the
/// same `RestrictedSchemeInfoBox` identifies: the track conforms to the
/// constraints of every scheme they indicate. The syntax and the semantics
/// are those of a [`SchemeTypeBox`].
///
/// `scheme_uri` is [`Some`] where the flag `0x000001` is set: the box is read
/// so wherever the flag is set, and written with the flag set where
/// `scheme_uri` is [`Some`]. The other flags are read past and written as
/// zero.
#[doc(alias = "csch")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct CompatibleSchemeTypeBox {
    /// Code of the scheme, normally a four character code
    pub scheme_type: FourCc,
    /// Version of the scheme used to create the content
    pub scheme_version: u32,
    /// Absolute URI of a web page for a user who does not have the scheme installed
    pub scheme_uri: Option<Utf8String>,
}

impl Default for CompatibleSchemeTypeBox {
    fn default() -> Self {
        Self {
            scheme_type: FourCc::new([0; 4]),
            scheme_version: 0,
            scheme_uri: None,
        }
    }
}

impl BoxDefinition for CompatibleSchemeTypeBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"csch");
}

impl BoxDecode for CompatibleSchemeTypeBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let SchemeTypeBox {
            scheme_type,
            scheme_version,
            scheme_uri,
        } = SchemeTypeBox::decode_fields(reader)?;

        Ok(Self {
            scheme_type,
            scheme_version,
            scheme_uri,
        })
    }
}

impl BoxEncode for CompatibleSchemeTypeBox {
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

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use isobmff_core::{BoxDecode, Error, FourCc, Utf8String};

    use super::CompatibleSchemeTypeBox;
    use crate::encoded;

    /// `csch` of the scheme `podv` version 1, of no URI
    const CSCH: &[u8] = b"\0\0\0\x14csch\0\0\0\0podv\0\0\0\x01";

    /// `csch` of the scheme `podv` version 1, of the URI `https://a.example/`
    const CSCH_URI: &[u8] = b"\0\0\0\x27csch\0\0\0\x01podv\0\0\0\x01https://a.example/\0";

    fn compatible_scheme_type(scheme_uri: Option<&str>) -> CompatibleSchemeTypeBox {
        CompatibleSchemeTypeBox {
            scheme_type: FourCc::new(*b"podv"),
            scheme_version: 1,
            scheme_uri: scheme_uri.map(|uri| Utf8String::new(String::from(uri)).unwrap()),
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            CompatibleSchemeTypeBox::decode(CSCH),
            Ok((compatible_scheme_type(None), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&compatible_scheme_type(None)), CSCH);
    }

    #[test]
    fn reads_the_uri_where_the_flag_is_set() {
        assert_eq!(
            CompatibleSchemeTypeBox::decode(CSCH_URI),
            Ok((
                compatible_scheme_type(Some("https://a.example/")),
                b"".as_slice()
            ))
        );
    }

    #[test]
    fn writes_the_flag_and_the_uri_where_there_is_one() {
        assert_eq!(
            encoded(&compatible_scheme_type(Some("https://a.example/"))),
            CSCH_URI
        );
    }

    #[test]
    fn a_payload_ending_inside_a_field_is_refused() {
        assert_eq!(
            CompatibleSchemeTypeBox::decode_payload(b"\0\0\0\0podv\0\0"),
            Err(Error::truncated_payload(12, 10))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            CompatibleSchemeTypeBox::decode_payload(b"\x01\0\0\0podv\0\0\0\x01"),
            Err(Error::unsupported_version(1))
        );
    }
}
