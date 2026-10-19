//! The muxers, [`FragmentedMuxer`], [`NonFragmentedMuxer`] and [`MediaSegmentMuxer`], and the parts they share: the output they make, and [`whole_payload`], [`whole_box_header`] and [`compact_box_header`], one box written whole into the steps it is laid down as

#[cfg(test)]
pub(crate) mod driving;
mod error;
mod media_segment;
mod movie;
mod output;

use alloc::vec;
use alloc::vec::Vec;

use isobmff_core::{BoxDefinition, BoxEncode, BoxHeader, BoxType, FieldWidth};

pub use error::MuxError;
pub use media_segment::MediaSegmentMuxer;
pub use movie::{FragmentedMuxer, NonFragmentedMuxer};

use output::MuxOutput;

/// Encodes `value` as the payload of the whole box it forms, naming that box on a failure
///
/// The mirror of [`WholeBoxReader`](crate::demux::WholeBoxReader): the payload comes back in one allocation
/// sized to what the value declares, for a muxer to lay down between the
/// header and the end of the box.
///
/// # Errors
///
/// * [`Box`](crate::MuxError::Box): the value does not write, or
///   declares a payload longer than any buffer on this target holds.
pub(crate) fn whole_payload<Value: BoxEncode + BoxDefinition>(
    value: &Value,
) -> Result<Vec<u8>, MuxError> {
    let payload_len = value.payload_len();
    let Ok(length) = usize::try_from(payload_len) else {
        return Err(past_every_buffer(Value::BOX_TYPE, payload_len));
    };
    let mut payload = vec![0; length];

    value
        .encode_payload(&mut payload)
        .map_err(|failure| failure.in_container(Value::BOX_TYPE))?;

    Ok(payload)
}

/// Returns the header of the whole box of `box_type` whose payload is `payload_len` bytes long
///
/// # Errors
///
/// * [`Box`](crate::MuxError::Box): no header measures a payload
///   that long, which no buffer on this target holds either.
pub(crate) fn whole_box_header(box_type: BoxType, payload_len: u64) -> Result<BoxHeader, MuxError> {
    BoxHeader::with_payload_len(box_type, payload_len)
        .ok_or_else(|| past_every_buffer(box_type, payload_len))
}

/// Returns the header of the whole box of `box_type` whose payload is `payload_len` bytes long, in the compact form alone
///
/// A payload only the `largesize` field can measure is refused, so a header
/// measured before its payload was whole keeps the length it was measured at.
///
/// # Errors
///
/// * [`Box`](crate::MuxError::Box): the total of header and
///   payload does not fit the 32 bits of the `size` field, reported as
///   [`OutOfRange`](crate::ErrorKind::OutOfRange) of the box with the
///   payload's length, not the total, as the value.
pub(crate) fn compact_box_header(
    box_type: BoxType,
    payload_len: u64,
) -> Result<BoxHeader, MuxError> {
    BoxHeader::compact(box_type, payload_len).ok_or_else(|| {
        isobmff_core::Error::out_of_range(payload_len, FieldWidth::Compact)
            .in_container(box_type)
            .into()
    })
}

/// Reports a box longer than any buffer on this target, as `isobmff-core` names it
fn past_every_buffer(box_type: BoxType, payload_len: u64) -> MuxError {
    // Why not a failure of its own: a payload past `usize`, and a header that
    // cannot measure one, both exceed every buffer this target can hold, which
    // is the short buffer `isobmff_core::BoxEncode::encode` folds them into.
    MuxError::from(
        isobmff_core::Error::truncated_buffer(payload_len, usize::MAX as u64)
            .in_container(box_type),
    )
}

#[cfg(test)]
mod tests {
    use isobmff_boxes::MediaDataBox;

    use super::{BoxDefinition, FieldWidth, compact_box_header};

    #[test]
    fn a_compact_header_is_refused_for_a_payload_only_the_extended_form_declares() {
        assert_eq!(
            compact_box_header(MediaDataBox::BOX_TYPE, u32::MAX.into()),
            Err(
                isobmff_core::Error::out_of_range(u32::MAX.into(), FieldWidth::Compact)
                    .in_container(MediaDataBox::BOX_TYPE)
                    .into()
            )
        );
    }
}
