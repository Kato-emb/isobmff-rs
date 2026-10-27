//! The compressed boxes, each holding the deflated payload of the top-level box it replaces

use alloc::vec::Vec;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

/// Returns the bytes of `compressed_data` that run to the end of the payload
fn read_compressed_data(reader: &mut FieldReader<'_>) -> Vec<u8> {
    reader.take_remainder().to_vec()
}

/// Returns the bytes `compressed_data` takes
fn compressed_data_len(compressed_data: &[u8]) -> u64 {
    u64::try_from(compressed_data.len()).unwrap_or(u64::MAX)
}

/// [`CompressedMovieBox`] (`!mov`), ISO/IEC 14496-12 §8.16.6
///
/// The payload of a [`MovieBox`](crate::MovieBox), compressed with deflate
/// (IETF RFC 1951), in place of that box: its replacement type is `moov`.
///
/// The box holds the compressed bytes as they are. Restoring the `moov`
/// they stand for is the layer that handles a whole file, by the processing
/// model of §8.16.2.
#[doc(alias = "!mov")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct CompressedMovieBox {
    /// Payload of the `moov` the box replaces, deflated
    pub compressed_data: Vec<u8>,
}

impl CompressedMovieBox {
    /// Box type of the box whose payload this box compresses
    pub const REPLACEMENT_TYPE: BoxType = BoxType::compact(*b"moov");
}

impl BoxDefinition for CompressedMovieBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"!mov");
}

impl BoxDecode for CompressedMovieBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            compressed_data: read_compressed_data(reader),
        })
    }
}

impl BoxEncode for CompressedMovieBox {
    fn payload_len(&self) -> u64 {
        compressed_data_len(&self.compressed_data)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_slice(&self.compressed_data)
    }
}

/// [`CompressedMovieFragmentBox`] (`!mof`), ISO/IEC 14496-12 §8.16.7
///
/// The payload of a [`MovieFragmentBox`](crate::MovieFragmentBox), compressed
/// with deflate (IETF RFC 1951), in place of that box: its replacement type
/// is `moof`.
///
/// The box holds the compressed bytes as they are. Restoring the `moof`
/// they stand for is the layer that handles a whole file, by the processing
/// model of §8.16.2.
#[doc(alias = "!mof")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct CompressedMovieFragmentBox {
    /// Payload of the `moof` the box replaces, deflated
    pub compressed_data: Vec<u8>,
}

impl CompressedMovieFragmentBox {
    /// Box type of the box whose payload this box compresses
    pub const REPLACEMENT_TYPE: BoxType = BoxType::compact(*b"moof");
}

impl BoxDefinition for CompressedMovieFragmentBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"!mof");
}

impl BoxDecode for CompressedMovieFragmentBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            compressed_data: read_compressed_data(reader),
        })
    }
}

impl BoxEncode for CompressedMovieFragmentBox {
    fn payload_len(&self) -> u64 {
        compressed_data_len(&self.compressed_data)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_slice(&self.compressed_data)
    }
}

/// [`CompressedSegmentIndexBox`] (`!six`), ISO/IEC 14496-12 §8.16.8
///
/// The payload of a [`SegmentIndexBox`](crate::SegmentIndexBox), compressed
/// with deflate (IETF RFC 1951), in place of that box: its replacement type
/// is `sidx`. The sizes and offsets in that payload count bytes of the file as
/// it is compressed.
///
/// The box holds the compressed bytes as they are. Restoring the `sidx`
/// they stand for, and moving its offsets to the decompressed file, is the
/// layer that handles a whole file, by the processing model of §8.16.2.
#[doc(alias = "!six")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct CompressedSegmentIndexBox {
    /// Payload of the `sidx` the box replaces, deflated
    pub compressed_data: Vec<u8>,
}

impl CompressedSegmentIndexBox {
    /// Box type of the box whose payload this box compresses
    pub const REPLACEMENT_TYPE: BoxType = BoxType::compact(*b"sidx");
}

impl BoxDefinition for CompressedSegmentIndexBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"!six");
}

impl BoxDecode for CompressedSegmentIndexBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            compressed_data: read_compressed_data(reader),
        })
    }
}

impl BoxEncode for CompressedSegmentIndexBox {
    fn payload_len(&self) -> u64 {
        compressed_data_len(&self.compressed_data)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_slice(&self.compressed_data)
    }
}

/// [`CompressedSubsegmentIndexBox`] (`!ssx`), ISO/IEC 14496-12 §8.16.9
///
/// The payload of a [`SubsegmentIndexBox`](crate::SubsegmentIndexBox),
/// compressed with deflate (IETF RFC 1951), in place of that box: its
/// replacement type is `ssix`. The sizes in that payload count bytes of the
/// file as it is compressed.
///
/// The box holds the compressed bytes as they are. Restoring the `ssix`
/// they stand for, and moving its sizes to the decompressed file, is the
/// layer that handles a whole file, by the processing model of §8.16.2.
#[doc(alias = "!ssx")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct CompressedSubsegmentIndexBox {
    /// Payload of the `ssix` the box replaces, deflated
    pub compressed_data: Vec<u8>,
}

impl CompressedSubsegmentIndexBox {
    /// Box type of the box whose payload this box compresses
    pub const REPLACEMENT_TYPE: BoxType = BoxType::compact(*b"ssix");
}

impl BoxDefinition for CompressedSubsegmentIndexBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"!ssx");
}

impl BoxDecode for CompressedSubsegmentIndexBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            compressed_data: read_compressed_data(reader),
        })
    }
}

impl BoxEncode for CompressedSubsegmentIndexBox {
    fn payload_len(&self) -> u64 {
        compressed_data_len(&self.compressed_data)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_slice(&self.compressed_data)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::BoxDecode;

    use super::{
        CompressedMovieBox, CompressedMovieFragmentBox, CompressedSegmentIndexBox,
        CompressedSubsegmentIndexBox,
    };
    use crate::encoded;

    /// `!mov` holding the deflated payload of an empty `moov`: a final stored block of no byte
    const COMPRESSED_MOVIE: &[u8] = b"\0\0\0\x0d!mov\x01\0\0\xff\xff";

    #[test]
    fn a_compressed_movie_box_reads_and_writes_its_compressed_data() {
        let compressed = CompressedMovieBox {
            compressed_data: vec![0x01, 0, 0, 0xff, 0xff],
        };

        assert_eq!(
            CompressedMovieBox::decode(COMPRESSED_MOVIE),
            Ok((compressed.clone(), b"".as_slice()))
        );
        assert_eq!(encoded(&compressed), COMPRESSED_MOVIE);
    }

    #[test]
    fn a_compressed_movie_fragment_box_reads_and_writes_its_compressed_data() {
        let bytes = b"\0\0\0\x0a!mof\xaa\xbb";
        let compressed = CompressedMovieFragmentBox {
            compressed_data: vec![0xaa, 0xbb],
        };

        assert_eq!(
            CompressedMovieFragmentBox::decode(bytes),
            Ok((compressed.clone(), b"".as_slice()))
        );
        assert_eq!(encoded(&compressed), bytes);
    }

    #[test]
    fn a_compressed_segment_index_box_reads_and_writes_its_compressed_data() {
        let bytes = b"\0\0\0\x0a!six\xaa\xbb";
        let compressed = CompressedSegmentIndexBox {
            compressed_data: vec![0xaa, 0xbb],
        };

        assert_eq!(
            CompressedSegmentIndexBox::decode(bytes),
            Ok((compressed.clone(), b"".as_slice()))
        );
        assert_eq!(encoded(&compressed), bytes);
    }

    #[test]
    fn a_compressed_subsegment_index_box_reads_and_writes_its_compressed_data() {
        let bytes = b"\0\0\0\x0a!ssx\xaa\xbb";
        let compressed = CompressedSubsegmentIndexBox {
            compressed_data: vec![0xaa, 0xbb],
        };

        assert_eq!(
            CompressedSubsegmentIndexBox::decode(bytes),
            Ok((compressed.clone(), b"".as_slice()))
        );
        assert_eq!(encoded(&compressed), bytes);
    }

    #[test]
    fn a_box_of_no_compressed_data_reads_and_writes_as_its_header_alone() {
        let empty = b"\0\0\0\x08!mov";

        assert_eq!(
            CompressedMovieBox::decode(empty),
            Ok((CompressedMovieBox::default(), b"".as_slice()))
        );
        assert_eq!(encoded(&CompressedMovieBox::default()), empty);
    }
}
