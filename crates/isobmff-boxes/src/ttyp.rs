//! [`TrackTypeBox`], the file type of a track

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FourCc,
};

use crate::ftyp::{general_type_len, read_general_type, write_general_type};

/// [`TrackTypeBox`] (`ttyp`), ISO/IEC 14496-12 §8.3.5
///
/// Identifies the specifications a track complies with, in the fields a
/// [`FileTypeBox`](crate::FileTypeBox) carries for a file: the brands that
/// would apply to the file were every other track removed but the ones this
/// track references.
///
/// [`Default`] is the box with every field zero and no compatible brand.
#[doc(alias = "ttyp")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TrackTypeBox {
    /// Brand of the specification that is the best use of the track
    pub major_brand: FourCc,
    /// Informative integer for the minor version of the major brand
    pub minor_version: u32,
    /// Brands of the specifications the track complies with
    pub compatible_brands: Vec<FourCc>,
}

impl Default for TrackTypeBox {
    fn default() -> Self {
        Self {
            major_brand: FourCc::new([0; 4]),
            minor_version: 0,
            compatible_brands: Vec::new(),
        }
    }
}

impl BoxDefinition for TrackTypeBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"ttyp");
}

impl BoxDecode for TrackTypeBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let (major_brand, minor_version, compatible_brands) = read_general_type(reader)?;

        Ok(Self {
            major_brand,
            minor_version,
            compatible_brands,
        })
    }
}

impl BoxEncode for TrackTypeBox {
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

    use super::TrackTypeBox;
    use crate::encoded;

    /// `ttyp` of major brand `cmf2`, minor version 0 and the compatible brands `cmf2` and `cmfc`
    const TTYP: &[u8] = b"\0\0\0\x18ttypcmf2\0\0\0\0cmf2cmfc";

    fn track_type() -> TrackTypeBox {
        TrackTypeBox {
            major_brand: FourCc::new(*b"cmf2"),
            minor_version: 0,
            compatible_brands: vec![FourCc::new(*b"cmf2"), FourCc::new(*b"cmfc")],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            TrackTypeBox::decode(TTYP),
            Ok((track_type(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&track_type()), TTYP);
    }

    #[test]
    fn a_payload_ending_inside_a_compatible_brand_is_refused() {
        assert_eq!(
            TrackTypeBox::decode_payload(b"cmf2\0\0\0\0cm"),
            Err(Error::truncated_payload(12, 10))
        );
    }
}
