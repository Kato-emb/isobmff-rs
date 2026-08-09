//! [`FileTypeBox`] (`ftyp`), ISO/IEC 14496-12 §4.3, and [`SegmentTypeBox`] (`styp`), §8.16.2

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, DecodeError, EncodeError, FourCC,
};

/// Reads the `major_brand`, `minor_version`, and `compatible_brands` of a payload
fn decode_brands(payload: &[u8]) -> Result<(FourCC, u32, Vec<FourCC>), DecodeError> {
    // Why not unwrap: a usize above `u64::MAX` needs a 128-bit target to exist,
    // and saturating keeps the panic-free path.
    let available = u64::try_from(payload.len()).unwrap_or(u64::MAX);
    let truncated_at = |needed| DecodeError::TruncatedPayload { needed, available };

    let (major_brand, after_major_brand) = payload
        .split_first_chunk::<4>()
        .ok_or_else(|| truncated_at(4))?;
    let (minor_version, after_minor_version) = after_major_brand
        .split_first_chunk::<4>()
        .ok_or_else(|| truncated_at(8))?;

    let mut compatible_brands = Vec::new();
    let mut remaining = after_minor_version;
    while let Some((brand, next)) = remaining.split_first_chunk::<4>() {
        compatible_brands.push(FourCC::new(*brand));
        remaining = next;
    }

    if !remaining.is_empty() {
        let shortfall = 4_u64.saturating_sub(u64::try_from(remaining.len()).unwrap_or(4));

        return Err(truncated_at(available.saturating_add(shortfall)));
    }

    Ok((
        FourCC::new(*major_brand),
        u32::from_be_bytes(*minor_version),
        compatible_brands,
    ))
}

/// Returns the length of a payload carrying the given brands
fn brands_payload_len(compatible_brands: &[FourCC]) -> u64 {
    u64::try_from(compatible_brands.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(4)
        .saturating_add(8)
}

/// Writes the shared fields into a payload buffer of exactly their length
fn encode_brands(
    major_brand: FourCC,
    minor_version: u32,
    compatible_brands: &[FourCC],
    buffer: &mut [u8],
) -> Result<(), EncodeError> {
    let expected = brands_payload_len(compatible_brands);
    let actual = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
    let mismatch = EncodeError::BufferLengthMismatch { expected, actual };
    if actual != expected {
        return Err(mismatch);
    }

    let (major_brand_field, after_major_brand) =
        buffer.split_first_chunk_mut::<4>().ok_or(mismatch)?;
    *major_brand_field = *major_brand.as_bytes();

    let (minor_version_field, after_minor_version) = after_major_brand
        .split_first_chunk_mut::<4>()
        .ok_or(mismatch)?;
    *minor_version_field = minor_version.to_be_bytes();

    let mut remaining = after_minor_version;
    for brand in compatible_brands {
        let (field, next) = remaining.split_first_chunk_mut::<4>().ok_or(mismatch)?;
        *field = *brand.as_bytes();
        remaining = next;
    }

    Ok(())
}

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

/// Box that declares the brands a segment complies with
///
/// [`SegmentTypeBox`] (`styp`), ISO/IEC 14496-12 §8.16.2. The fields are those
/// of [`FileTypeBox`], whose syntax the spec defines it to share. A segment
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

    use isobmff_core::{BoxDecode, BoxWrite as _, DecodeError, FourCC};

    use super::{FileTypeBox, SegmentTypeBox};

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

        let payload = encoded(&file_type);

        assert_eq!(
            FileTypeBox::decode_payload(payload.get(8..).unwrap()).unwrap(),
            file_type
        );
    }

    #[test]
    fn a_payload_ending_inside_the_minor_version_is_rejected_as_truncated() {
        assert!(matches!(
            FileTypeBox::decode_payload(b"isom\0\0"),
            Err(DecodeError::TruncatedPayload {
                needed: 8,
                available: 6
            })
        ));
    }

    #[test]
    fn a_compatible_brand_cut_short_names_the_length_that_would_complete_it() {
        assert!(matches!(
            FileTypeBox::decode_payload(b"isom\0\0\0\0iso"),
            Err(DecodeError::TruncatedPayload {
                needed: 12,
                available: 11
            })
        ));
    }

    #[test]
    fn the_segment_box_reads_the_same_payload_as_the_file_box_under_its_own_type() {
        let payload = b"iso6\0\0\x02\0dash";

        let file_type = FileTypeBox::decode_payload(payload).unwrap();
        let segment_type = SegmentTypeBox::decode_payload(payload).unwrap();

        assert_eq!(segment_type.major_brand(), file_type.major_brand());
        assert_eq!(segment_type.minor_version(), file_type.minor_version());
        assert_eq!(
            segment_type.compatible_brands(),
            file_type.compatible_brands()
        );
    }

    #[test]
    fn the_segment_box_writes_itself_under_the_styp_code() {
        let segment_type = SegmentTypeBox::new(FourCC::new(*b"msdh"), 0, Vec::new());
        let mut buffer = vec![0; usize::try_from(segment_type.encoded_len()).unwrap()];

        segment_type.encode(&mut buffer).unwrap();

        assert_eq!(buffer, b"\0\0\0\x10stypmsdh\0\0\0\0");
    }
}
