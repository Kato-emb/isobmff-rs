//! [`UriInitBox`], the initialization data of the metadata of a track

use alloc::vec::Vec;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::full_box::{read_version_zero, write_version_zero};

/// [`UriInitBox`] (`uriI`), ISO/IEC 14496-12 §12.3.3
///
/// Initialization data of the timed metadata of a
/// [`UriMetaSampleEntry`](crate::UriMetaSampleEntry), opaque, of the form the
/// documentation of its URI defines.
///
/// The box reads version 0 alone. The `flags` are read past and written as
/// zero.
#[doc(alias = "uriI", alias = "URIInitBox")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct UriInitBox {
    /// Initialization data, of the form the documentation of the URI defines
    pub uri_initialization_data: Vec<u8>,
}

impl BoxDefinition for UriInitBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"uriI");
}

impl BoxDecode for UriInitBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {
            uri_initialization_data: reader.take_remainder().to_vec(),
        })
    }
}

impl BoxEncode for UriInitBox {
    fn payload_len(&self) -> u64 {
        u64::try_from(self.uri_initialization_data.len())
            .unwrap_or(u64::MAX)
            .saturating_add(4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        writer.write_slice(&self.uri_initialization_data)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, Error};

    use super::UriInitBox;
    use crate::encoded;

    /// `uriI` of three bytes of initialization data
    const URII: &[u8] = b"\0\0\0\x0furiI\0\0\0\0\x01\x02\x03";

    fn uri_init() -> UriInitBox {
        UriInitBox {
            uri_initialization_data: vec![1, 2, 3],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(UriInitBox::decode(URII), Ok((uri_init(), b"".as_slice())));
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&uri_init()), URII);
    }

    #[test]
    fn a_version_other_than_0_is_refused() {
        assert_eq!(
            UriInitBox::decode_payload(b"\x01\0\0\0\x01"),
            Err(Error::unsupported_version(1))
        );
    }
}
