//! [`SegmentTypeBox`] (`styp`), ISO/IEC 14496-12 §8.16.2

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FourCC,
};

/// Length of the fields that precede the compatible brands
const FIXED_FIELDS_LEN: u64 = 8;

/// Length one brand occupies
const BRAND_LEN: u64 = 4;

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
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends inside a field, which includes a `compatible_brands` list
    ///   whose length is not a multiple of four.
    fn decode_payload(payload: &[u8]) -> Result<Self, Error> {
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

impl BoxEncode for SegmentTypeBox {
    fn payload_len(&self) -> u64 {
        u64::try_from(self.compatible_brands.len())
            .unwrap_or(u64::MAX)
            .saturating_mul(BRAND_LEN)
            .saturating_add(FIXED_FIELDS_LEN)
    }

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), Error> {
        let expected = self.payload_len();
        let actual = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        if actual != expected {
            return Err(Error::buffer_length_mismatch(expected, actual));
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

    use isobmff_core::{BoxDecode, BoxWrite as _, Error, FourCC};

    use super::SegmentTypeBox;

    #[test]
    fn a_segment_reads_its_brands_under_its_own_type() {
        assert_eq!(
            SegmentTypeBox::decode_payload(b"iso6\0\0\x02\0dash").unwrap(),
            SegmentTypeBox::new(FourCC::new(*b"iso6"), 512, vec![FourCC::new(*b"dash")])
        );
    }

    #[test]
    fn a_compatible_brand_cut_short_names_the_length_that_would_complete_it() {
        assert_eq!(
            SegmentTypeBox::decode_payload(b"msdh\0\0\0\0msd"),
            Err(Error::truncated_payload(12, 11))
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
