//! [`MetadataSetupBox`], the setup data a metadata key needs

use alloc::vec::Vec;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

/// [`MetadataSetupBox`] (`setu`), ISO/IEC 14496-12 §12.8.4.6
///
/// Setup data the values of a metadata key need, laid out as the
/// `key_namespace` of the key defines: leaf data, or an array of boxes. The
/// box holds it as its bytes.
///
/// Of the namespaces ISO/IEC 14496-12 defines, `uiso` uses no setup data, and
/// `me4c` uses the bytes that follow `data_reference_index` in a sample entry
/// of the format its `key_value` names, which a
/// [`SampleEntry`](crate::SampleEntry) of that format holding these bytes
/// reads.
#[doc(alias = "setu")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct MetadataSetupBox {
    /// Setup data, as the namespace of the key lays it out
    pub data: Vec<u8>,
}

impl BoxDefinition for MetadataSetupBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"setu");
}

impl BoxDecode for MetadataSetupBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            data: reader.take_remainder().to_vec(),
        })
    }
}

impl BoxEncode for MetadataSetupBox {
    fn payload_len(&self) -> u64 {
        u64::try_from(self.data.len()).unwrap_or(u64::MAX)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_slice(&self.data)
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::BoxDecode;

    use super::MetadataSetupBox;
    use crate::encoded;

    /// `setu` of the extensions of a `metx` entry: no content encoding, the namespace `urn:x` and no schema
    const SETU: &[u8] = b"\0\0\0\x10setu\0urn:x\0\0";

    fn setup() -> MetadataSetupBox {
        MetadataSetupBox {
            data: b"\0urn:x\0\0".to_vec(),
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            MetadataSetupBox::decode(SETU),
            Ok((setup(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&setup()), SETU);
    }
}
