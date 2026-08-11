//! [`FileTypeBox`] (`ftyp`), ISO/IEC 14496-12 §4.3

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, DecodeError, EncodeError, FieldReader,
    FieldWriter, FourCC,
};

/// Length of the fields that precede the compatible brands
const FIXED_FIELDS_LEN: u64 = 8;

/// Length one brand occupies
const BRAND_LEN: u64 = 4;

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
    /// * [`Field`](DecodeError::Field): the payload ends inside a field, which
    ///   includes a `compatible_brands` list whose length is not a multiple of
    ///   four.
    fn decode_payload(payload: &[u8]) -> Result<Self, DecodeError> {
        let mut reader = FieldReader::new(payload);
        let major_brand = FourCC::new(*reader.read_bytes::<4>()?);
        let minor_version = reader.read_u32()?;

        let mut compatible_brands = Vec::new();
        while !reader.remainder().is_empty() {
            compatible_brands.push(FourCC::new(*reader.read_bytes::<4>()?));
        }

        Ok(Self::new(major_brand, minor_version, compatible_brands))
    }
}

impl BoxEncode for FileTypeBox {
    fn payload_len(&self) -> u64 {
        u64::try_from(self.compatible_brands.len())
            .unwrap_or(u64::MAX)
            .saturating_mul(BRAND_LEN)
            .saturating_add(FIXED_FIELDS_LEN)
    }

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), EncodeError> {
        let expected = self.payload_len();
        let actual = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        if actual != expected {
            return Err(EncodeError::BufferLengthMismatch { expected, actual });
        }

        let mut writer = FieldWriter::new(buffer);
        writer.write_bytes(self.major_brand.as_bytes())?;
        writer.write_u32(self.minor_version)?;
        for brand in &self.compatible_brands {
            writer.write_bytes(brand.as_bytes())?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{
        BoxDecode, BoxEncode as _, BoxWrite as _, DecodeError, EncodeError, FieldReadError, FourCC,
    };

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

    #[test]
    fn a_payload_reads_into_the_fields_the_spec_lays_out_in_order() {
        assert_eq!(
            FileTypeBox::decode_payload(b"iso6\0\0\x02\0iso6dash").unwrap(),
            FileTypeBox::new(
                FourCC::new(*b"iso6"),
                512,
                vec![FourCC::new(*b"iso6"), FourCC::new(*b"dash")]
            )
        );
    }

    #[test]
    fn a_payload_ending_inside_the_minor_version_is_rejected_as_truncated() {
        assert!(matches!(
            FileTypeBox::decode_payload(b"isom\0\0"),
            Err(DecodeError::Field(FieldReadError::UnexpectedEof {
                needed: 8,
                available: 6
            }))
        ));
    }

    #[test]
    fn a_compatible_brand_cut_short_names_the_length_that_would_complete_it() {
        assert!(matches!(
            FileTypeBox::decode_payload(b"isom\0\0\0\0iso"),
            Err(DecodeError::Field(FieldReadError::UnexpectedEof {
                needed: 12,
                available: 11
            }))
        ));
    }

    #[test]
    fn a_buffer_with_room_to_spare_is_refused_as_a_short_one_is() {
        let file_type = FileTypeBox::new(FourCC::new(*b"isom"), 0, Vec::new());

        assert_eq!(
            file_type.encode_payload(&mut [0; 32]),
            Err(EncodeError::BufferLengthMismatch {
                expected: 8,
                actual: 32
            })
        );
    }

    #[test]
    fn every_compatible_brand_adds_four_bytes_to_the_fixed_fields() {
        let none = FileTypeBox::new(FourCC::new(*b"isom"), 0, Vec::new());
        let two = FileTypeBox::new(
            FourCC::new(*b"isom"),
            0,
            vec![FourCC::new(*b"isom"), FourCC::new(*b"iso6")],
        );

        assert_eq!(none.payload_len(), 8);
        assert_eq!(two.payload_len(), 16);
    }
}
