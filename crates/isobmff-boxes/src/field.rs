//! Fixed-width fields a box payload is made of, split off its front

use isobmff_core::{DecodeError, EncodeError};

/// Splits the next fixed-width field off the front of `rest`
///
/// `needed` and `available` describe the payload as a whole rather than `rest`,
/// so a box that has already settled its length reports the same failure from
/// every field it reads.
pub(crate) fn split_field<const WIDTH: usize>(
    rest: &[u8],
    needed: u64,
    available: u64,
) -> Result<(&[u8; WIDTH], &[u8]), DecodeError> {
    rest.split_first_chunk::<WIDTH>()
        .ok_or(DecodeError::TruncatedPayload { needed, available })
}

/// Splits the next fixed-width field off the front of `rest` to write into
pub(crate) fn split_field_mut<const WIDTH: usize>(
    rest: &mut [u8],
    mismatch: EncodeError,
) -> Result<(&mut [u8; WIDTH], &mut [u8]), EncodeError> {
    rest.split_first_chunk_mut::<WIDTH>().ok_or(mismatch)
}

/// Reports a payload that is not the exact length a box of fixed width settled on
pub(crate) fn check_payload_len(needed: u64, available: u64) -> Result<(), DecodeError> {
    if available < needed {
        return Err(DecodeError::TruncatedPayload { needed, available });
    }
    if available > needed {
        return Err(DecodeError::TrailingBytes {
            remaining: available.saturating_sub(needed),
        });
    }

    Ok(())
}

/// Splits off a time field, which is 32 bits wide at version 0 and 64 at version 1
pub(crate) fn split_time(
    version: u8,
    rest: &[u8],
    needed: u64,
    available: u64,
) -> Result<(u64, &[u8]), DecodeError> {
    if version == 0 {
        let (field, rest) = split_field::<4>(rest, needed, available)?;

        return Ok((u64::from(u32::from_be_bytes(*field)), rest));
    }

    let (field, rest) = split_field::<8>(rest, needed, available)?;

    Ok((u64::from_be_bytes(*field), rest))
}

/// Writes a time field at the width `version` selects
pub(crate) fn write_time(
    version: u8,
    value: u64,
    rest: &mut [u8],
    mismatch: EncodeError,
) -> Result<&mut [u8], EncodeError> {
    if version == 0 {
        let (field, rest) = split_field_mut::<4>(rest, mismatch)?;
        // Why not unwrap: a box picks version 0 only for times that fit, so this
        // conversion holds; the fallback keeps the panic-free path.
        *field = u32::try_from(value).map_err(|_| mismatch)?.to_be_bytes();

        return Ok(rest);
    }

    let (field, rest) = split_field_mut::<8>(rest, mismatch)?;
    *field = value.to_be_bytes();

    Ok(rest)
}

/// Splits off an array of 32-bit signed fields laid end to end
pub(crate) fn split_i32_array<const COUNT: usize>(
    rest: &[u8],
    needed: u64,
    available: u64,
) -> Result<([i32; COUNT], &[u8]), DecodeError> {
    let mut values = [0; COUNT];
    let mut remaining = rest;
    for value in &mut values {
        let (field, next) = split_field::<4>(remaining, needed, available)?;
        *value = i32::from_be_bytes(*field);
        remaining = next;
    }

    Ok((values, remaining))
}

/// Writes an array of 32-bit signed fields end to end
pub(crate) fn write_i32_array<'buffer>(
    values: &[i32],
    rest: &'buffer mut [u8],
    mismatch: EncodeError,
) -> Result<&'buffer mut [u8], EncodeError> {
    let mut remaining = rest;
    for value in values {
        let (field, next) = split_field_mut::<4>(remaining, mismatch)?;
        *field = value.to_be_bytes();
        remaining = next;
    }

    Ok(remaining)
}

/// Splits off an array of 32-bit unsigned fields laid end to end
pub(crate) fn split_u32_array<const COUNT: usize>(
    rest: &[u8],
    needed: u64,
    available: u64,
) -> Result<([u32; COUNT], &[u8]), DecodeError> {
    let mut values = [0; COUNT];
    let mut remaining = rest;
    for value in &mut values {
        let (field, next) = split_field::<4>(remaining, needed, available)?;
        *value = u32::from_be_bytes(*field);
        remaining = next;
    }

    Ok((values, remaining))
}

/// Writes an array of 32-bit unsigned fields end to end
pub(crate) fn write_u32_array<'buffer>(
    values: &[u32],
    rest: &'buffer mut [u8],
    mismatch: EncodeError,
) -> Result<&'buffer mut [u8], EncodeError> {
    let mut remaining = rest;
    for value in values {
        let (field, next) = split_field_mut::<4>(remaining, mismatch)?;
        *field = value.to_be_bytes();
        remaining = next;
    }

    Ok(remaining)
}
