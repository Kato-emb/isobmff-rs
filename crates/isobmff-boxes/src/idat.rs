//! [`ItemDataBox`], the data of items stored in a meta box

use alloc::vec::Vec;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

/// [`ItemDataBox`] (`idat`), ISO/IEC 14496-12 §8.11.11
///
/// The data of the items whose extents the `ItemLocationBox` places in this
/// box, by the construction method 1. Their offsets count from the first byte
/// of [`data`](Self::data).
#[doc(alias = "idat")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct ItemDataBox {
    /// Data of the items
    pub data: Vec<u8>,
}

impl BoxDefinition for ItemDataBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"idat");
}

impl BoxDecode for ItemDataBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            data: reader.take_remainder().to_vec(),
        })
    }
}

impl BoxEncode for ItemDataBox {
    fn payload_len(&self) -> u64 {
        u64::try_from(self.data.len()).unwrap_or(u64::MAX)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_slice(&self.data)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::BoxDecode;

    use super::ItemDataBox;
    use crate::encoded;

    /// `idat` of three bytes of item data
    const IDAT: &[u8] = b"\0\0\0\x0bidat\xaa\xbb\xcc";

    fn item_data() -> ItemDataBox {
        ItemDataBox {
            data: vec![0xaa, 0xbb, 0xcc],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(ItemDataBox::decode(IDAT), Ok((item_data(), b"".as_slice())));
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&item_data()), IDAT);
    }
}
