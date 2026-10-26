//! [`BrandProperty`], the brands an item complies with

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FourCc,
};

use crate::ftyp::{general_type_len, read_general_type, write_general_type};

/// [`BrandProperty`] (`brnd`), ISO/IEC 14496-12 §8.11.15
///
/// A descriptive item property that identifies the specifications an item
/// complies with, in the fields a [`FileTypeBox`](crate::FileTypeBox) carries
/// for a file: the brands that would apply to the file were every track and
/// every item removed but this item and the items it references.
///
/// [`Default`] is the box with every field zero and no compatible brand.
#[doc(alias = "brnd")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct BrandProperty {
    /// Brand of the specification that is the best use of the item
    pub major_brand: FourCc,
    /// Informative integer for the minor version of the major brand
    pub minor_version: u32,
    /// Brands of the specifications the item complies with
    pub compatible_brands: Vec<FourCc>,
}

impl Default for BrandProperty {
    fn default() -> Self {
        Self {
            major_brand: FourCc::new([0; 4]),
            minor_version: 0,
            compatible_brands: Vec::new(),
        }
    }
}

impl BoxDefinition for BrandProperty {
    const BOX_TYPE: BoxType = BoxType::compact(*b"brnd");
}

impl BoxDecode for BrandProperty {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let (major_brand, minor_version, compatible_brands) = read_general_type(reader)?;

        Ok(Self {
            major_brand,
            minor_version,
            compatible_brands,
        })
    }
}

impl BoxEncode for BrandProperty {
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

    use super::BrandProperty;
    use crate::encoded;

    /// `brnd` of major brand `heic`, minor version 0 and the compatible brands `mif1` and `heic`
    const BRND: &[u8] = b"\0\0\0\x18brndheic\0\0\0\0mif1heic";

    fn brand() -> BrandProperty {
        BrandProperty {
            major_brand: FourCc::new(*b"heic"),
            minor_version: 0,
            compatible_brands: vec![FourCc::new(*b"mif1"), FourCc::new(*b"heic")],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(BrandProperty::decode(BRND), Ok((brand(), b"".as_slice())));
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&brand()), BRND);
    }

    #[test]
    fn a_payload_ending_inside_a_compatible_brand_is_refused() {
        assert_eq!(
            BrandProperty::decode_payload(b"heic\0\0\0\0mi"),
            Err(Error::truncated_payload(12, 10))
        );
    }
}
