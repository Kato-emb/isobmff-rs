//! [`OriginalFormatBox`], the format of a sample entry before it was transformed

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FourCc,
};

/// [`OriginalFormatBox`] (`frma`), ISO/IEC 14496-12 §13.4.3
///
/// The four character code of the sample entry before it was transformed:
/// the format of the decrypted, encoded data of a protected sample entry, or
/// the format of the un-transformed sample entry of a restricted sample entry
/// or of a sample entry for an incomplete track.
#[doc(alias = "frma")]
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct OriginalFormatBox {
    /// Four character code of the original un-transformed sample entry, such as `mp4v`
    pub data_format: FourCc,
}

impl Default for OriginalFormatBox {
    fn default() -> Self {
        Self {
            data_format: FourCc::new([0; 4]),
        }
    }
}

impl BoxDefinition for OriginalFormatBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"frma");
}

impl BoxDecode for OriginalFormatBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            data_format: FourCc::new(*reader.read_bytes::<4>()?),
        })
    }
}

impl BoxEncode for OriginalFormatBox {
    fn payload_len(&self) -> u64 {
        4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(self.data_format.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, Error, FourCc};

    use super::OriginalFormatBox;
    use crate::encoded;

    /// `frma` of the format `avc1`
    const FRMA: &[u8] = b"\0\0\0\x0cfrmaavc1";

    fn original_format() -> OriginalFormatBox {
        OriginalFormatBox {
            data_format: FourCc::new(*b"avc1"),
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            OriginalFormatBox::decode(FRMA),
            Ok((original_format(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&original_format()), FRMA);
    }

    #[test]
    fn a_payload_ending_inside_the_format_is_refused() {
        assert_eq!(
            OriginalFormatBox::decode_payload(b"avc"),
            Err(Error::truncated_payload(4, 3))
        );
    }
}
