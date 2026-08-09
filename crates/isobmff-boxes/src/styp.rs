//! [`SegmentTypeBox`] (`styp`), ISO/IEC 14496-12 §8.16.2

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, DecodeError, EncodeError, FourCC,
};

use crate::brand::{brands_payload_len, decode_brands, encode_brands};

/// Box that declares the brands a segment complies with
///
/// [`SegmentTypeBox`] (`styp`), ISO/IEC 14496-12 §8.16.2. The fields are those
/// of [`FileTypeBox`](crate::FileTypeBox), whose syntax the spec defines it
/// to share. A segment
/// carries one where a whole file would carry an `ftyp`, so a reader that has
/// only the segment can still tell what it may treat it as.
#[doc(alias = "styp")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SegmentTypeBox {
    major_brand: FourCC,
    minor_version: u32,
    compatible_brands: Vec<FourCC>,
}

impl SegmentTypeBox {
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

    /// Returns the brand naming the specification the segment was written to
    #[must_use]
    pub const fn major_brand(&self) -> FourCC {
        self.major_brand
    }

    /// Returns the revision of the specification the `major_brand` names
    #[must_use]
    pub const fn minor_version(&self) -> u32 {
        self.minor_version
    }

    /// Returns every brand a reader may treat the segment as
    #[must_use]
    pub fn compatible_brands(&self) -> &[FourCC] {
        &self.compatible_brands
    }
}

impl BoxDefinition for SegmentTypeBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"styp");
}

impl BoxDecode for SegmentTypeBox {
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

impl BoxEncode for SegmentTypeBox {
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

    use super::SegmentTypeBox;

    #[test]
    fn a_segment_reads_its_brands_under_its_own_type() {
        assert_eq!(
            SegmentTypeBox::decode_payload(b"iso6\0\0\x02\0dash").unwrap(),
            SegmentTypeBox::new(FourCC::new(*b"iso6"), 512, vec![FourCC::new(*b"dash")])
        );
    }

    #[test]
    fn a_segment_writes_itself_under_the_styp_code() {
        let segment_type = SegmentTypeBox::new(FourCC::new(*b"msdh"), 0, Vec::new());
        let mut buffer = vec![0; usize::try_from(segment_type.encoded_len()).unwrap()];

        segment_type.encode(&mut buffer).unwrap();

        assert_eq!(buffer, b"\0\0\0\x10stypmsdh\0\0\0\0");
    }
}
