//! [`KindBox`], the role or kind of a track

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, Utf8String,
};

use crate::full_box::{read_version_zero, write_version_zero};

/// [`KindBox`] (`kind`), ISO/IEC 14496-12 §8.10.4
///
/// Role or kind of a track: a URI that names the kind, or a URI that names a
/// scheme and a value from it. A track may hold several, of different
/// contents.
///
/// `value` is [`None`] where no byte follows the terminator of `scheme_uri`,
/// and is written so; an empty string after it reads as `Some` of the empty
/// string. The box reads version 0 alone. The `flags` are read past and
/// written as zero.
#[doc(alias = "kind")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct KindBox {
    /// URI that names the kind where no value follows, and the scheme of `value` where one does
    pub scheme_uri: Utf8String,
    /// Name of the kind in the scheme `scheme_uri` names
    pub value: Option<Utf8String>,
}

impl BoxDefinition for KindBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"kind");
}

impl BoxDecode for KindBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;
        let mut strings = reader.take_remainder().splitn(2, |byte| *byte == 0);

        Ok(Self {
            scheme_uri: Utf8String::from_slice(strings.next().unwrap_or_default()),
            value: strings
                .next()
                .filter(|value| !value.is_empty())
                .map(Utf8String::from_slice),
        })
    }
}

impl BoxEncode for KindBox {
    fn payload_len(&self) -> u64 {
        self.value
            .as_ref()
            .map_or(0, Utf8String::encoded_len)
            .saturating_add(self.scheme_uri.encoded_len())
            .saturating_add(4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        let rest = self.scheme_uri.encode(writer.take_remainder())?;
        if let Some(value) = &self.value {
            value.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use isobmff_core::{BoxDecode, Error, Utf8String};

    use super::KindBox;
    use crate::encoded;

    /// `kind` of the scheme `urn:mpeg:dash:role:2011` and the value `main`
    const KIND: &[u8] = b"\0\0\0\x29kind\0\0\0\0urn:mpeg:dash:role:2011\0main\0";

    fn string(text: &str) -> Utf8String {
        Utf8String::new(String::from(text)).unwrap()
    }

    fn kind() -> KindBox {
        KindBox {
            scheme_uri: string("urn:mpeg:dash:role:2011"),
            value: Some(string("main")),
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(KindBox::decode(KIND), Ok((kind(), b"".as_slice())));
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&kind()), KIND);
    }

    #[test]
    fn a_kind_named_by_its_uri_alone_holds_no_value() {
        let bytes = b"\0\0\0\x18kind\0\0\0\0urn:example\0";
        let named = KindBox {
            scheme_uri: string("urn:example"),
            value: None,
        };

        assert_eq!(
            (KindBox::decode(bytes), encoded(&named)),
            (Ok((named.clone(), b"".as_slice())), bytes.to_vec())
        );
    }

    #[test]
    fn an_empty_value_is_kept_and_written_back() {
        let bytes = b"\0\0\0\x19kind\0\0\0\0urn:example\0\0";
        let empty = KindBox {
            scheme_uri: string("urn:example"),
            value: Some(Utf8String::default()),
        };

        assert_eq!(
            (KindBox::decode(bytes), encoded(&empty)),
            (Ok((empty.clone(), b"".as_slice())), bytes.to_vec())
        );
    }

    #[test]
    fn a_version_other_than_0_is_refused() {
        assert_eq!(
            KindBox::decode_payload(b"\x01\0\0\0urn\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
