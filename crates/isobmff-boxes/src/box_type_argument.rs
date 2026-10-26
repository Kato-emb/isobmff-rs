//! Writing a box whose box type is a value it holds, the argument of a class whose box type takes values no list closes
//!
//! Such a box implements [`BoxEncode`] for its payload, but not
//! [`BoxDefinition`](isobmff_core::BoxDefinition), since no one box type names
//! it. The container holding it writes the header from the value.

use isobmff_core::{BoxEncode, BoxHeader, BoxType, CompactType, Error, FourCc};

use crate::free::{Free, FreeType};

/// Returns the number of bytes the box of `payload` takes, header included
pub(crate) fn encoded_len(payload: &impl BoxEncode) -> u64 {
    let payload_len = payload.payload_len();
    // Why not the box type the value holds: the length of a compact header
    // does not depend on its four-character code, and a `uuid` code, which
    // `encode` refuses, has no compact header to measure.
    BoxHeader::with_payload_len(Free::FREE_TYPE, payload_len).map_or(u64::MAX, |header| {
        header.encoded_len().saturating_add(payload_len)
    })
}

/// Writes `payload` as a box of the box type `box_type` onto the front of `buffer`, and returns what is left
///
/// # Errors
///
/// * [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) naming `field`:
///   `box_type` is `uuid`, which a compact header cannot carry.
/// * [`TruncatedBuffer`](isobmff_core::ErrorKind::TruncatedBuffer): `buffer`
///   is shorter than [`encoded_len`].
/// * The failures of the payload's [`BoxEncode::encode_payload`].
pub(crate) fn encode<'buffer>(
    box_type: FourCc,
    field: &'static str,
    payload: &impl BoxEncode,
    buffer: &'buffer mut [u8],
) -> Result<&'buffer mut [u8], Error> {
    let compact = CompactType::new(box_type).ok_or(Error::invalid_value(field))?;
    let payload_len = payload.payload_len();
    let header = BoxHeader::with_payload_len(BoxType::Compact(compact), payload_len);
    let needed = header.map_or(u64::MAX, |header| {
        header.encoded_len().saturating_add(payload_len)
    });
    let available = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
    let too_short = || Error::truncated_buffer(needed, available);
    let header = header.ok_or_else(too_short)?;
    // Why not report a total beyond `usize` as its own error: such a total
    // exceeds any `buffer.len()` on the same target, so it is a short buffer.
    let (whole, rest) = usize::try_from(needed)
        .ok()
        .and_then(|needed| buffer.split_at_mut_checked(needed))
        .ok_or_else(too_short)?;
    payload.encode_payload(header.encode(whole)?)?;

    Ok(rest)
}
