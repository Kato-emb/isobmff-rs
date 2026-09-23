//! Tables of 4-bit values packed two to a byte, as the `padb` of ISO/IEC 14496-12 §8.7.6 and the `stz2` of §8.7.3.3 lay them out

use alloc::vec::Vec;

use isobmff_core::{Error, FieldWriter};

/// Mask of the low half of a byte
const LOW_HALF: u8 = 0x0f;

/// Returns the values `packed` holds, the earlier of each pair in the high half of its byte
///
/// The low half of the last byte of a table of an odd number of values holds
/// none and is left out, which leaves `declared` values when the bytes are as
/// many as that table takes; any other count comes out as the bytes hold it,
/// for the caller to check against `declared`.
pub(crate) fn unpack(packed: &[u8], declared: u64) -> Vec<u8> {
    // Why not the low half first, as GPAC's C implementation packs a `padb`
    // pair: §8.7.6 lays `pad1` out ahead of `pad2` and §8.7.3.3 writes
    // `entry[i]<<4 + entry[i+1]`, so a `padb` GPAC wrote reads here with each
    // pair swapped, and its last entry lost when the count is odd.
    let mut values: Vec<u8> = packed
        .iter()
        .flat_map(|byte| [byte >> 4, byte & LOW_HALF])
        .collect();
    if values.len() as u64 == declared.saturating_add(1) {
        values.pop();
    }

    values
}

/// Writes `values`, each under 16, two to a byte, the earlier of each pair in the high half
///
/// The low half of the last byte is written as zero when the values are odd in
/// number.
pub(crate) fn pack(
    writer: &mut FieldWriter<'_>,
    values: impl IntoIterator<Item = u8>,
) -> Result<(), Error> {
    let mut values = values.into_iter();
    while let Some(first) = values.next() {
        let second = values.next().unwrap_or(0);
        writer.write_bytes(&[first << 4 | second])?;
    }

    Ok(())
}
