//! [`ChunkOffsetBox`]

use alloc::vec::Vec;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::full_box::{read_version_zero, write_version_zero};
use crate::table::{counted_len, read_counted, write_counted};

/// [`ChunkOffsetBox`] (`stco`), ISO/IEC 14496-12 §8.7.5
///
/// The offset of the start of each chunk, in 32 bits. An offset counts from
/// the start of the file, or, where the data reference entry of the chunk is a
/// `DataEntryImdaBox` or a `DataEntrySeqNumImdaBox`, from the first byte of the
/// payload of the `IdentifiedMediaDataBox` it names.
#[doc(alias = "stco")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct ChunkOffsetBox {
    /// `chunk_offset` of every chunk, in chunk order
    pub chunk_offsets: Vec<u32>,
}

impl BoxDefinition for ChunkOffsetBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"stco");
}

impl BoxDecode for ChunkOffsetBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {
            chunk_offsets: read_counted(reader)?,
        })
    }
}

impl BoxEncode for ChunkOffsetBox {
    fn payload_len(&self) -> u64 {
        counted_len(&self.chunk_offsets).saturating_add(4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        write_counted(writer, &self.chunk_offsets)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, Error};

    use super::ChunkOffsetBox;
    use crate::encoded;

    /// `stco` of chunks at 48 and at `0x1_0000`
    const STCO: &[u8] = b"\0\0\0\x18stco\0\0\0\0\0\0\0\x02\0\0\0\x30\0\x01\0\0";

    fn chunk_offset() -> ChunkOffsetBox {
        ChunkOffsetBox {
            chunk_offsets: vec![48, 0x1_0000],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            ChunkOffsetBox::decode(STCO),
            Ok((chunk_offset(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&chunk_offset()), STCO);
    }

    #[test]
    fn a_payload_ending_inside_an_offset_is_refused() {
        assert_eq!(
            ChunkOffsetBox::decode_payload(b"\0\0\0\0\0\0\0\x01\0\0"),
            Err(Error::truncated_payload(12, 10))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            ChunkOffsetBox::decode_payload(b"\x01\0\0\0\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
