//! [`SegmentTypeBox`], the file type of a segment

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FourCc,
};

use crate::ftyp::{general_type_len, read_general_type, write_general_type};

/// [`SegmentTypeBox`] (`styp`), ISO/IEC 14496-12 §8.14.2
///
/// Identifies the specifications a segment complies with, in the fields a
/// [`FileTypeBox`](crate::FileTypeBox) carries for a file.
///
/// [`Default`] is the box with every field zero and no compatible brand.
#[doc(alias = "styp")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SegmentTypeBox {
    /// Brand of the specification that is the best use of the segment
    pub major_brand: FourCc,
    /// Informative integer for the minor version of the major brand
    pub minor_version: u32,
    /// Brands of the specifications the segment complies with
    pub compatible_brands: Vec<FourCc>,
}

impl Default for SegmentTypeBox {
    fn default() -> Self {
        Self {
            major_brand: FourCc::new([0; 4]),
            minor_version: 0,
            compatible_brands: Vec::new(),
        }
    }
}

impl BoxDefinition for SegmentTypeBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"styp");
}

impl BoxDecode for SegmentTypeBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let (major_brand, minor_version, compatible_brands) = read_general_type(reader)?;

        Ok(Self {
            major_brand,
            minor_version,
            compatible_brands,
        })
    }
}

impl BoxEncode for SegmentTypeBox {
    fn payload_len(&self) -> u64 {
        general_type_len(&self.compatible_brands)
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

    use super::SegmentTypeBox;
    use crate::encoded;

    /// `styp` of major brand `msdh`, minor version 0 and the compatible brands `msdh` and `msix`
    const STYP: &[u8] = b"\0\0\0\x18stypmsdh\0\0\0\0msdhmsix";

    fn segment_type() -> SegmentTypeBox {
        SegmentTypeBox {
            major_brand: FourCc::new(*b"msdh"),
            minor_version: 0,
            compatible_brands: vec![FourCc::new(*b"msdh"), FourCc::new(*b"msix")],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            SegmentTypeBox::decode(STYP),
            Ok((segment_type(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&segment_type()), STYP);
    }

    #[test]
    fn a_payload_ending_inside_the_major_brand_is_refused() {
        assert_eq!(
            SegmentTypeBox::decode_payload(b"ms"),
            Err(Error::truncated_payload(4, 2))
        );
    }
}
