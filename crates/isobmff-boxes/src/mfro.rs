//! [`MovieFragmentRandomAccessOffsetBox`], the size of a movie fragment random access box, repeated at its end

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::full_box::{read_version_zero, write_version_zero};

/// [`MovieFragmentRandomAccessOffsetBox`] (`mfro`), ISO/IEC 14496-12 §8.8.11
///
/// A copy of the size of the `MovieFragmentRandomAccessBox` around it, placed
/// last in that box so that a reader scanning back from the end of the file
/// finds it.
///
/// The box holds `parent_size` as it was read: a writer of the file sets it
/// to the size of the box around it.
#[doc(alias = "mfro")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct MovieFragmentRandomAccessOffsetBox {
    /// Number of bytes of the enclosing `MovieFragmentRandomAccessBox`
    pub parent_size: u32,
}

impl BoxDefinition for MovieFragmentRandomAccessOffsetBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mfro");
}

impl BoxDecode for MovieFragmentRandomAccessOffsetBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {
            parent_size: reader.read_u32()?,
        })
    }
}

impl BoxEncode for MovieFragmentRandomAccessOffsetBox {
    fn payload_len(&self) -> u64 {
        4 + 4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        writer.write_u32(self.parent_size)
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, Error};

    use super::MovieFragmentRandomAccessOffsetBox;
    use crate::encoded;

    /// `mfro` of a parent size of 114
    const MFRO: &[u8] = b"\0\0\0\x10mfro\0\0\0\0\0\0\0\x72";

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            MovieFragmentRandomAccessOffsetBox::decode(MFRO),
            Ok((
                MovieFragmentRandomAccessOffsetBox { parent_size: 114 },
                b"".as_slice()
            ))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            encoded(&MovieFragmentRandomAccessOffsetBox { parent_size: 114 }),
            MFRO
        );
    }

    #[test]
    fn a_payload_ending_inside_the_parent_size_is_refused() {
        assert_eq!(
            MovieFragmentRandomAccessOffsetBox::decode_payload(b"\0\0\0\0\0\0"),
            Err(Error::truncated_payload(8, 6))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            MovieFragmentRandomAccessOffsetBox::decode_payload(b"\x01\0\0\0\0\0\0\x72"),
            Err(Error::unsupported_version(1))
        );
    }
}
