//! [`MetadataKeyDeclarationBox`], the namespace and value of a metadata key

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FourCc,
};

/// [`MetadataKeyDeclarationBox`] (`keyd`), ISO/IEC 14496-12 §12.8.4.4
///
/// The key a `MetadataKeyBox` declares: a namespace, and a value whose layout
/// the namespace defines, such as the user-data four-character code of the
/// namespace `uiso`.
///
/// [`Default`] is the box of `key_namespace` four zero bytes and an empty
/// `key_value`.
#[doc(alias = "keyd")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct MetadataKeyDeclarationBox {
    /// Domain and structure of the key value, such as `uiso` or `me4c`
    pub key_namespace: FourCc,
    /// Key, laid out as the namespace defines
    pub key_value: Vec<u8>,
}

impl Default for MetadataKeyDeclarationBox {
    fn default() -> Self {
        Self {
            key_namespace: FourCc::new([0; 4]),
            key_value: Vec::new(),
        }
    }
}

impl BoxDefinition for MetadataKeyDeclarationBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"keyd");
}

impl BoxDecode for MetadataKeyDeclarationBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            key_namespace: FourCc::new(*reader.read_bytes::<4>()?),
            key_value: reader.take_remainder().to_vec(),
        })
    }
}

impl BoxEncode for MetadataKeyDeclarationBox {
    fn payload_len(&self) -> u64 {
        u64::try_from(self.key_value.len())
            .unwrap_or(u64::MAX)
            .saturating_add(4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(self.key_namespace.as_bytes())?;
        writer.write_slice(&self.key_value)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, Error, FourCc};

    use super::MetadataKeyDeclarationBox;
    use crate::encoded;

    /// `keyd` of the namespace `uiso` and the user-data key `cprt`
    const KEYD: &[u8] = b"\0\0\0\x10keyduisocprt";

    fn declaration() -> MetadataKeyDeclarationBox {
        MetadataKeyDeclarationBox {
            key_namespace: FourCc::new(*b"uiso"),
            key_value: vec![b'c', b'p', b'r', b't'],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            MetadataKeyDeclarationBox::decode(KEYD),
            Ok((declaration(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&declaration()), KEYD);
    }

    #[test]
    fn a_payload_ending_inside_the_namespace_is_refused() {
        assert_eq!(
            MetadataKeyDeclarationBox::decode_payload(b"uis"),
            Err(Error::truncated_payload(4, 3))
        );
    }
}
