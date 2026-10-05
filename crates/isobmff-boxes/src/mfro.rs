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
/// It is the last box of its `mfra`, so when the `mfra` is also the last box
/// of the file a reader finds it by reading this box off the last
/// [`ENCODED_LEN`](Self::ENCODED_LEN) bytes and stepping back `size` bytes
/// from the end.
///
/// Neither the version nor the `flags` are held — the spec defines only
/// version 0 and declares the flags zero for this box.
#[doc(alias = "mfro")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct MovieFragmentRandomAccessOffsetBox {
    size: u32,
}

impl MovieFragmentRandomAccessOffsetBox {
    /// Bytes the box occupies in a file, its header included
    ///
    /// An `mfro` whose size is stated in 32 bits occupies this many; a file
    /// closing with an `mfra` closes with them.
    pub const ENCODED_LEN: usize = 16;

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

    /// Returns the offset at which the enclosing `mfra` begins in a file `file_len` bytes long
    ///
    /// When the `mfra` is the last box of the file, it begins
    /// [`size`](Self::size) bytes before the end of the file. `None` when
    /// `size` exceeds `file_len`. Whether an `mfra` stands at the returned
    /// offset is not checked.
    #[must_use]
    pub fn movie_fragment_random_access_start(&self, file_len: u64) -> Option<u64> {
        file_len.checked_sub(u64::from(self.size))
    }
}

impl BoxDefinition for MovieFragmentRandomAccessOffsetBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mfro");
}

impl BoxDecode for MovieFragmentRandomAccessOffsetBox {
    type Error = Error;

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
    fn the_mfra_begins_size_bytes_before_the_end_of_the_file() {
        let offset = MovieFragmentRandomAccessOffsetBox::new(100);

        assert_eq!(offset.movie_fragment_random_access_start(1000), Some(900));
        assert_eq!(offset.movie_fragment_random_access_start(100), Some(0));
    }

    #[test]
    fn a_size_past_the_start_of_the_file_locates_no_mfra() {
        let offset = MovieFragmentRandomAccessOffsetBox::new(100);

        assert_eq!(offset.movie_fragment_random_access_start(99), None);
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
