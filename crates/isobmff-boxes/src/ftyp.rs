//! [`FileTypeBox`], and the fields every box of the `GeneralTypeBox` class carries

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FourCc,
};

use crate::field::{FieldCodec, entries_len, read_to_end, write_entries};

/// Fields of a `GeneralTypeBox`, as `major_brand`, `minor_version` and `compatible_brands`
pub(crate) type GeneralTypeFields = (FourCc, u32, Vec<FourCc>);

/// Reads the fields of a `GeneralTypeBox`, which run to the end of the box
///
/// # Errors
///
/// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
///   payload ends inside a field or inside a brand.
pub(crate) fn read_general_type(reader: &mut FieldReader<'_>) -> Result<GeneralTypeFields, Error> {
    Ok((
        FourCc::read(reader)?,
        u32::read(reader)?,
        read_to_end(reader)?,
    ))
}

/// Returns how many bytes the fields of a `GeneralTypeBox` take, saturating at `u64::MAX`
pub(crate) fn general_type_len(
    major_brand: FourCc,
    minor_version: u32,
    compatible_brands: &[FourCc],
) -> u64 {
    major_brand
        .encoded_len()
        .saturating_add(minor_version.encoded_len())
        .saturating_add(entries_len(compatible_brands))
}

/// Writes the fields of a `GeneralTypeBox`
///
/// # Errors
///
/// * [`TruncatedBuffer`](isobmff_core::ErrorKind::TruncatedBuffer): the
///   buffer ends inside a field.
pub(crate) fn write_general_type(
    writer: &mut FieldWriter<'_>,
    major_brand: FourCc,
    minor_version: u32,
    compatible_brands: &[FourCc],
) -> Result<(), Error> {
    major_brand.write(writer)?;
    minor_version.write(writer)?;
    write_entries(writer, compatible_brands)
}

/// [`FileTypeBox`] (`ftyp`), ISO/IEC 14496-12 §5.2
///
/// Identifies the specification that is the best use of the file, by its
/// `major_brand`, and the specifications the file complies with, by its
/// `compatible_brands`.
///
/// [`Default`] is the box with every field zero and no compatible brand.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::FileTypeBox;
/// use isobmff_core::{BoxDecode, BoxEncode, FourCc};
///
/// // A box is built from its default, field by field
/// let mut file_type = FileTypeBox::default();
/// file_type.major_brand = FourCc::new(*b"isom");
/// file_type.minor_version = 0x200;
/// file_type.compatible_brands = vec![FourCc::new(*b"isom"), FourCc::new(*b"iso2")];
///
/// // It writes the bytes its syntax lays out, and reads back from them
/// let mut buffer = [0; 24];
/// file_type.encode(&mut buffer).unwrap();
/// assert_eq!(&buffer, b"\0\0\0\x18ftypisom\0\0\x02\0isomiso2");
/// assert_eq!(FileTypeBox::decode(&buffer), Ok((file_type, b"".as_slice())));
/// ```
#[doc(alias = "ftyp")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct FileTypeBox {
    /// Brand of the specification that is the best use of the file
    pub major_brand: FourCc,
    /// Informative integer for the minor version of the major brand
    pub minor_version: u32,
    /// Brands of the specifications the file complies with
    pub compatible_brands: Vec<FourCc>,
}

impl Default for FileTypeBox {
    fn default() -> Self {
        Self {
            major_brand: FourCc::new([0; 4]),
            minor_version: 0,
            compatible_brands: Vec::new(),
        }
    }
}

impl BoxDefinition for FileTypeBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"ftyp");
}

impl BoxDecode for FileTypeBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let (major_brand, minor_version, compatible_brands) = read_general_type(reader)?;

        Ok(Self {
            major_brand,
            minor_version,
            compatible_brands,
        })
    }
}

impl BoxEncode for FileTypeBox {
    fn payload_len(&self) -> u64 {
        general_type_len(
            self.major_brand,
            self.minor_version,
            &self.compatible_brands,
        )
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_general_type(
            writer,
            self.major_brand,
            self.minor_version,
            &self.compatible_brands,
        )
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, Error, FourCc};

    use super::FileTypeBox;
    use crate::encoded;

    /// `ftyp` of major brand `isom`, minor version 0x200 and the compatible brands `isom` and `iso2`
    const FTYP: &[u8] = b"\0\0\0\x18ftypisom\0\0\x02\0isomiso2";

    fn file_type() -> FileTypeBox {
        FileTypeBox {
            major_brand: FourCc::new(*b"isom"),
            minor_version: 0x200,
            compatible_brands: vec![FourCc::new(*b"isom"), FourCc::new(*b"iso2")],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(FileTypeBox::decode(FTYP), Ok((file_type(), b"".as_slice())));
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&file_type()), FTYP);
    }

    #[test]
    fn a_box_of_no_compatible_brand_reads_as_an_empty_list() {
        assert_eq!(
            FileTypeBox::decode_payload(b"mp41\0\0\0\0"),
            Ok(FileTypeBox {
                major_brand: FourCc::new(*b"mp41"),
                minor_version: 0,
                compatible_brands: vec![],
            })
        );
    }

    #[test]
    fn a_payload_ending_inside_the_minor_version_is_refused() {
        assert_eq!(
            FileTypeBox::decode_payload(b"isom\0\0"),
            Err(Error::truncated_payload(8, 6))
        );
    }

    #[test]
    fn a_payload_ending_inside_a_compatible_brand_is_refused() {
        assert_eq!(
            FileTypeBox::decode_payload(b"isom\0\0\0\0is"),
            Err(Error::truncated_payload(12, 10))
        );
    }
}
