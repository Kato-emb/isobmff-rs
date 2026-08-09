//! Fields shared by the boxes that declare brands, ISO/IEC 14496-12 §4.3

use alloc::vec::Vec;

use isobmff_core::{DecodeError, EncodeError, FourCC};

/// Reads the `major_brand`, `minor_version`, and `compatible_brands` of a payload
pub(crate) fn decode_brands(payload: &[u8]) -> Result<(FourCC, u32, Vec<FourCC>), DecodeError> {
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
pub(crate) fn brands_payload_len(compatible_brands: &[FourCC]) -> u64 {
    u64::try_from(compatible_brands.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(4)
        .saturating_add(8)
}

/// Writes the shared fields into a payload buffer of exactly their length
pub(crate) fn encode_brands(
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

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{DecodeError, EncodeError, FourCC};

    use super::{brands_payload_len, decode_brands, encode_brands};

    #[test]
    fn a_payload_reads_into_the_fields_the_spec_lays_out_in_order() {
        assert_eq!(
            decode_brands(b"iso6\0\0\x02\0iso6dash").unwrap(),
            (
                FourCC::new(*b"iso6"),
                512,
                vec![FourCC::new(*b"iso6"), FourCC::new(*b"dash")]
            )
        );
    }

    #[test]
    fn a_payload_ending_inside_the_minor_version_is_rejected_as_truncated() {
        assert!(matches!(
            decode_brands(b"isom\0\0"),
            Err(DecodeError::TruncatedPayload {
                needed: 8,
                available: 6
            })
        ));
    }

    #[test]
    fn a_compatible_brand_cut_short_names_the_length_that_would_complete_it() {
        assert!(matches!(
            decode_brands(b"isom\0\0\0\0iso"),
            Err(DecodeError::TruncatedPayload {
                needed: 12,
                available: 11
            })
        ));
    }

    #[test]
    fn a_buffer_with_room_to_spare_is_refused_as_a_short_one_is() {
        assert_eq!(
            encode_brands(FourCC::new(*b"isom"), 0, &[], &mut [0; 32]),
            Err(EncodeError::BufferLengthMismatch {
                expected: 8,
                actual: 32
            })
        );
    }

    #[test]
    fn every_compatible_brand_adds_four_bytes_to_the_fixed_fields() {
        assert_eq!(brands_payload_len(&[]), 8);
        assert_eq!(
            brands_payload_len(&[FourCC::new(*b"isom"), FourCC::new(*b"iso6")]),
            16
        );
    }
}
