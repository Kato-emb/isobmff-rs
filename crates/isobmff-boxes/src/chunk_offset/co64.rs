//! [`ChunkLargeOffsetBox`]

use alloc::vec::Vec;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::full_box::{read_version_zero, write_version_zero};
use crate::table::{counted_len, read_counted, write_counted};

/// [`ChunkLargeOffsetBox`] (`co64`), ISO/IEC 14496-12 §8.7.5
///
/// The offset of the start of each chunk, in 64 bits, counted as in a
/// [`ChunkOffsetBox`](crate::ChunkOffsetBox).
#[doc(alias = "co64")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct ChunkLargeOffsetBox {
    /// `chunk_offset` of every chunk, in chunk order
    pub chunk_offsets: Vec<u64>,
}

impl BoxDefinition for ChunkLargeOffsetBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"co64");
}

impl BoxDecode for ChunkLargeOffsetBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {
            chunk_offsets: read_counted(reader)?,
        })
    }
}

impl BoxEncode for ChunkLargeOffsetBox {
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

    use super::ChunkLargeOffsetBox;
    use crate::encoded;

    /// `co64` of chunks at 48 and at `0x1_0000_0000`
    const CO64: &[u8] = b"\0\0\0\x20co64\0\0\0\0\0\0\0\x02\0\0\0\0\0\0\0\x30\0\0\0\x01\0\0\0\0";

    fn chunk_large_offset() -> ChunkLargeOffsetBox {
        ChunkLargeOffsetBox {
            chunk_offsets: vec![48, 0x1_0000_0000],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            ChunkLargeOffsetBox::decode(CO64),
            Ok((chunk_large_offset(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&chunk_large_offset()), CO64);
    }

    #[test]
    fn a_payload_ending_inside_an_offset_is_refused() {
        assert_eq!(
            ChunkLargeOffsetBox::decode_payload(b"\0\0\0\0\0\0\0\x01\0\0\0\0"),
            Err(Error::truncated_payload(16, 12))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            ChunkLargeOffsetBox::decode_payload(b"\x01\0\0\0\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
