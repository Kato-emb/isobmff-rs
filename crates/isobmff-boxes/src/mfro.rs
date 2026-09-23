//! [`MovieFragmentRandomAccessOffsetBox`] (`mfro`), ISO/IEC 14496-12 §8.8.11

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFields,
    FullBoxFlags,
};

/// Length of the payload, which has no version-dependent field
const PAYLOAD_LEN: u64 = 8;

/// Box that closes an `mfra` with the number of bytes the `mfra` occupies
///
/// [`MovieFragmentRandomAccessOffsetBox`] (`mfro`), ISO/IEC 14496-12 §8.8.11.
/// It is the last box of its `mfra`, and the `mfra` is the last box of the
/// file, so a reader finds the `mfra` by reading this box off the last 16
/// bytes of the file and stepping back `size` bytes from its end.
///
/// Neither the version nor the `flags` are held — the spec declares both zero
/// for this box.
#[doc(alias = "mfro")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct MovieFragmentRandomAccessOffsetBox {
    size: u32,
}

impl MovieFragmentRandomAccessOffsetBox {
    /// Creates the box from the number of bytes its `mfra` occupies
    #[must_use]
    pub const fn new(size: u32) -> Self {
        Self { size }
    }

    /// Returns the number of bytes the enclosing `mfra` occupies, this box included
    #[must_use]
    pub const fn size(&self) -> u32 {
        self.size
    }
}

impl BoxDefinition for MovieFragmentRandomAccessOffsetBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mfro");
}

impl BoxDecode for MovieFragmentRandomAccessOffsetBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
    ///   ends inside a field of the box.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version != 0 {
            return Err(Error::unsupported_version(version));
        }

        let size = reader.read_u32()?;

        Ok(Self { size })
    }
}

impl BoxEncode for MovieFragmentRandomAccessOffsetBox {
    fn payload_len(&self) -> u64 {
        PAYLOAD_LEN
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxFields::new(0, FullBoxFlags::ZERO).to_bytes())?;
        writer.write_u32(self.size)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::MovieFragmentRandomAccessOffsetBox;

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let offset = MovieFragmentRandomAccessOffsetBox::new(0x0102_0304);
        let mut payload = vec![0; 8];

        offset.encode_payload(&mut payload).unwrap();

        assert_eq!(payload, b"\0\0\0\0\x01\x02\x03\x04");
        assert_eq!(
            MovieFragmentRandomAccessOffsetBox::decode_payload(&payload).unwrap(),
            offset
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let mut payload = vec![0; 8];
        *payload.first_mut().unwrap() = 1;

        assert_eq!(
            MovieFragmentRandomAccessOffsetBox::decode_payload(&payload),
            Err(Error::unsupported_version(1))
        );
    }
}
