//! [`FileTypeBox`] (`ftyp`), ISO/IEC 14496-12 §4.3

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, DecodeError, EncodeError, FourCC,
};

use crate::brand::{brands_payload_len, decode_brands, encode_brands};

/// Box that declares the brands a file complies with
///
/// [`FileTypeBox`] (`ftyp`), ISO/IEC 14496-12 §4.3. The `major_brand` names the
/// specification the file was written to and the `minor_version` its revision,
/// while `compatible_brands` lists every specification a reader may treat the
/// file as. A file carries one, ahead of everything else in it.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::FileTypeBox;
/// use isobmff_core::{BoxDecode, BoxWrite, FourCC};
///
/// // The brands of a fragmented MP4 file
/// let file_type = FileTypeBox::new(
///     FourCC::new(*b"iso6"),
///     512,
///     vec![FourCC::new(*b"iso6"), FourCC::new(*b"dash")],
/// );
///
/// // The box writes to the bytes a file opens with
/// let mut buffer = vec![0; usize::try_from(file_type.encoded_len()).unwrap()];
/// file_type.encode(&mut buffer).unwrap();
/// assert_eq!(buffer, b"\0\0\0\x18ftypiso6\0\0\x02\0iso6dash");
///
/// // And reads back from them
/// assert_eq!(FileTypeBox::decode_payload(&buffer[8..]).unwrap(), file_type);
/// ```
#[doc(alias = "ftyp")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct FileTypeBox {
    major_brand: FourCC,
    minor_version: u32,
    compatible_brands: Vec<FourCC>,
}

impl FileTypeBox {
    /// Creates the box from the brands it declares
    #[must_use]
    pub const fn new(
        major_brand: FourCC,
        minor_version: u32,
        compatible_brands: Vec<FourCC>,
    ) -> Self {
        Self {
            major_brand,
            minor_version,
            compatible_brands,
        }
    }

    /// Returns the brand naming the specification the file was written to
    #[must_use]
    pub const fn major_brand(&self) -> FourCC {
        self.major_brand
    }

    /// Returns the revision of the specification the `major_brand` names
    #[must_use]
    pub const fn minor_version(&self) -> u32 {
        self.minor_version
    }

    /// Returns every brand a reader may treat the file as
    #[must_use]
    pub fn compatible_brands(&self) -> &[FourCC] {
        &self.compatible_brands
    }
}

impl BoxDefinition for FileTypeBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"ftyp");
}

impl BoxDecode for FileTypeBox {
    /// # Errors
    ///
    /// * [`TruncatedPayload`](DecodeError::TruncatedPayload): the payload ends
    ///   inside a field, which includes a `compatible_brands` list whose length
    ///   is not a multiple of four.
    fn decode_payload(payload: &[u8]) -> Result<Self, DecodeError> {
        let (major_brand, minor_version, compatible_brands) = decode_brands(payload)?;

        Ok(Self::new(major_brand, minor_version, compatible_brands))
    }
}

impl BoxEncode for FileTypeBox {
    fn payload_len(&self) -> u64 {
        brands_payload_len(&self.compatible_brands)
    }

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), EncodeError> {
        encode_brands(
            self.major_brand,
            self.minor_version,
            &self.compatible_brands,
            buffer,
        )
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxWrite as _, FourCC};

    use super::FileTypeBox;

    /// Writes the whole box and returns the bytes it occupies
    fn encoded(file_type: &FileTypeBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(file_type.encoded_len()).unwrap()];
        file_type.encode(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_declaring_no_compatible_brands_holds_only_the_two_fixed_fields() {
        let file_type = FileTypeBox::new(FourCC::new(*b"isom"), 0, Vec::new());

        assert_eq!(encoded(&file_type), b"\0\0\0\x10ftypisom\0\0\0\0");
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let file_type = FileTypeBox::new(
            FourCC::new(*b"iso6"),
            512,
            vec![FourCC::new(*b"iso6"), FourCC::new(*b"dash")],
        );

        let whole = encoded(&file_type);

        assert_eq!(
            FileTypeBox::decode_payload(whole.get(8..).unwrap()).unwrap(),
            file_type
        );
    }
}
