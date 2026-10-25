//! [`MediaDataBox`], the media data of a presentation

use alloc::vec::Vec;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

/// [`MediaDataBox`] (`mdat`), ISO/IEC 14496-12 §8.1.1
///
/// Media data, which the structure-data refers to by its absolute offset in
/// the file. The payload is held whole, so a box read holds a copy of all the
/// media data it frames.
#[doc(alias = "mdat")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct MediaDataBox {
    /// Media data the box contains
    pub data: Vec<u8>,
}

impl BoxDefinition for MediaDataBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mdat");
}

impl BoxDecode for MediaDataBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            data: reader.take_remainder().to_vec(),
        })
    }
}

impl BoxEncode for MediaDataBox {
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

    use super::MediaDataBox;
    use crate::encoded;

    /// `mdat` of three bytes of media data
    const MDAT: &[u8] = b"\0\0\0\x0bmdat\xaa\xbb\xcc";

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            MediaDataBox::decode(MDAT),
            Ok((
                MediaDataBox {
                    data: vec![0xaa, 0xbb, 0xcc]
                },
                b"".as_slice()
            ))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            encoded(&MediaDataBox {
                data: vec![0xaa, 0xbb, 0xcc]
            }),
            MDAT
        );
    }

    #[test]
    fn a_box_running_to_the_end_of_the_file_holds_the_rest_of_the_input() {
        assert_eq!(
            MediaDataBox::decode(b"\0\0\0\0mdat\x01\x02"),
            Ok((MediaDataBox { data: vec![1, 2] }, b"".as_slice()))
        );
    }
}
