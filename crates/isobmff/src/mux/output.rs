//! The bytes a muxer writes: each whole box it appends to the bytes it holds, and the header of an `mdat`

use alloc::vec::Vec;

use isobmff_core::{BoxDefinition, BoxEncode, BoxHeader, BoxSize, BoxType, ExtendedSize};

use super::Error;

/// Returns the header of an `mdat` of `payload_length` bytes, in the 16 bytes of a `largesize`, or `None` where the box would be longer than 64 bits count
///
/// The `media data` box, ISO/IEC 14496-12 §8.1.1, under the header of §4.2.2
/// with `size` 1: 16 bytes whatever the length, so a header written before
/// the payload is written over in place once the length is known.
pub(super) fn media_data_header(payload_length: u64) -> Option<[u8; 16]> {
    let size = ExtendedSize::new(payload_length.checked_add(16)?)?;
    let header = BoxHeader::new(BoxType::compact(*b"mdat"), BoxSize::Extended(size))?;
    let mut bytes = [0; 16];
    header.encode(&mut bytes).ok()?;
    Some(bytes)
}

/// Appends the whole box `value` forms to `out`, leaving `out` as it was where it does not write
pub(super) fn append_encoded(
    value: &(impl BoxEncode + BoxDefinition),
    out: &mut Vec<u8>,
) -> Result<(), Error> {
    let start = out.len();
    let length = usize::try_from(value.encoded_len()).map_err(|_| Error::Overflow)?;
    out.resize(start.saturating_add(length), 0);
    match value.encode(out.get_mut(start..).unwrap_or_default()) {
        Ok(_) => Ok(()),
        Err(source) => {
            out.truncate(start);
            Err(Error::Encode(source))
        }
    }
}
