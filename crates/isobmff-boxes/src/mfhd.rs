//! [`MovieFragmentHeaderBox`], the sequence number of a movie fragment

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::full_box::{read_version_zero, write_version_zero};

/// [`MovieFragmentHeaderBox`] (`mfhd`), ISO/IEC 14496-12 §8.8.5
///
/// The sequence number of a movie fragment, as a safety check: it usually
/// starts at 1 and increases for each movie fragment in the file, in the
/// order they occur.
#[doc(alias = "mfhd")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct MovieFragmentHeaderBox {
    /// Number associated with the movie fragment
    pub sequence_number: u32,
}

impl BoxDefinition for MovieFragmentHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mfhd");
}

impl BoxDecode for MovieFragmentHeaderBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {
            sequence_number: reader.read_u32()?,
        })
    }
}

impl BoxEncode for MovieFragmentHeaderBox {
    fn payload_len(&self) -> u64 {
        4 + 4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        writer.write_u32(self.sequence_number)
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, Error};

    use super::MovieFragmentHeaderBox;
    use crate::encoded;

    /// `mfhd` of sequence number 7
    const MFHD: &[u8] = b"\0\0\0\x10mfhd\0\0\0\0\0\0\0\x07";

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            MovieFragmentHeaderBox::decode(MFHD),
            Ok((
                MovieFragmentHeaderBox { sequence_number: 7 },
                b"".as_slice()
            ))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            encoded(&MovieFragmentHeaderBox { sequence_number: 7 }),
            MFHD
        );
    }

    #[test]
    fn a_payload_ending_inside_the_sequence_number_is_refused() {
        assert_eq!(
            MovieFragmentHeaderBox::decode_payload(b"\0\0\0\0\0\0"),
            Err(Error::truncated_payload(8, 6))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            MovieFragmentHeaderBox::decode_payload(b"\x01\0\0\0\0\0\0\x07"),
            Err(Error::unsupported_version(1))
        );
    }
}
