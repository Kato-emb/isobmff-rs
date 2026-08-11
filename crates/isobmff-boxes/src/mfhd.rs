//! [`MovieFragmentHeaderBox`] (`mfhd`), ISO/IEC 14496-12 §8.8.5

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, DecodeError, EncodeError, FieldReader,
    FieldWriter, FullBoxFields, FullBoxFlags,
};

/// Length of the payload, which has no version-dependent field
const PAYLOAD_LEN: u64 = 8;

/// Box that numbers one movie fragment among the fragments of a file
///
/// [`MovieFragmentHeaderBox`] (`mfhd`), ISO/IEC 14496-12 §8.8.5. The
/// `sequence_number` usually starts at 1 and increases with every fragment, in
/// the order the fragments occur, so a reader can tell that one arrived out of
/// order or went missing. A `moof` carries exactly one.
///
/// Neither the version nor the `flags` are held — the spec declares both zero
/// for this box.
#[doc(alias = "mfhd")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct MovieFragmentHeaderBox {
    sequence_number: u32,
}

impl MovieFragmentHeaderBox {
    /// Creates the box from the number it gives its fragment
    #[must_use]
    pub const fn new(sequence_number: u32) -> Self {
        Self { sequence_number }
    }

    /// Returns the number this fragment carries against the other fragments
    #[must_use]
    pub const fn sequence_number(&self) -> u32 {
        self.sequence_number
    }
}

impl BoxDefinition for MovieFragmentHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mfhd");
}

impl BoxDecode for MovieFragmentHeaderBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](DecodeError::UnsupportedVersion): the box
    ///   declares a version other than 0.
    /// * [`Field`](DecodeError::Field): the payload ends inside a field, or
    ///   holds bytes past the fields of the box.
    fn decode_payload(payload: &[u8]) -> Result<Self, DecodeError> {
        let mut reader = FieldReader::new(payload);
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version != 0 {
            return Err(DecodeError::UnsupportedVersion(version));
        }

        let sequence_number = reader.read_u32()?;
        reader.finish()?;

        Ok(Self { sequence_number })
    }
}

impl BoxEncode for MovieFragmentHeaderBox {
    fn payload_len(&self) -> u64 {
        PAYLOAD_LEN
    }

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), EncodeError> {
        let actual = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        if actual != PAYLOAD_LEN {
            return Err(EncodeError::BufferLengthMismatch {
                expected: PAYLOAD_LEN,
                actual,
            });
        }

        let mut writer = FieldWriter::new(buffer);
        writer.write_bytes(&FullBoxFields::new(0, FullBoxFlags::ZERO).to_bytes())?;
        writer.write_u32(self.sequence_number)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, DecodeError, FieldReadError};

    use super::MovieFragmentHeaderBox;

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let movie_fragment_header = MovieFragmentHeaderBox::new(7);
        let mut payload = vec![0; 8];

        movie_fragment_header.encode_payload(&mut payload).unwrap();

        assert_eq!(payload, b"\0\0\0\0\0\0\0\x07");
        assert_eq!(
            MovieFragmentHeaderBox::decode_payload(&payload).unwrap(),
            movie_fragment_header
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let mut payload = vec![0; 8];
        *payload.first_mut().unwrap() = 1;

        assert!(matches!(
            MovieFragmentHeaderBox::decode_payload(&payload),
            Err(DecodeError::UnsupportedVersion(1))
        ));
    }

    #[test]
    fn a_payload_shorter_than_the_fields_is_rejected() {
        assert!(matches!(
            MovieFragmentHeaderBox::decode_payload(&[0; 7]),
            Err(DecodeError::Field(FieldReadError::UnexpectedEof {
                needed: 8,
                available: 7
            }))
        ));
    }
}
