//! [`BoxWrite`], the whole box of ISO/IEC 14496-12 §4.2 written from a value

use crate::codec::box_definition::BoxDefinition;
use crate::codec::box_encode::BoxEncode;
use crate::error::{Error, byte_count};
use crate::framing::box_header::BoxHeader;
use crate::framing::box_type::BoxType;

/// Returns the length of the whole box `payload` forms under `box_type`
pub(crate) fn encoded_len_of(box_type: BoxType, payload: &(impl BoxEncode + ?Sized)) -> u64 {
    let payload_len = payload.payload_len();

    match BoxHeader::with_payload_len(box_type, payload_len) {
        Some(header) => u64::try_from(header.encoded_len())
            .unwrap_or(u64::MAX)
            .saturating_add(payload_len),
        None => u64::MAX,
    }
}

/// Writes the whole box `payload` forms under `box_type` into `buffer`
pub(crate) fn encode_into<'buffer>(
    box_type: BoxType,
    payload: &(impl BoxEncode + ?Sized),
    buffer: &'buffer mut [u8],
) -> Result<&'buffer mut [u8], Error> {
    let needed = encoded_len_of(box_type, payload);
    let too_short = Error::truncated_buffer(needed, byte_count(buffer.len()));

    let header = BoxHeader::with_payload_len(box_type, payload.payload_len()).ok_or(too_short)?;
    let mut scratch = [0; BoxHeader::MAX_ENCODED_LEN];
    let encoded_header = header.encode(&mut scratch);

    // Why not report a total beyond `usize` as its own error: such a total
    // exceeds any `buffer.len()` on the same target, so it is a short buffer by
    // another name and folding it in keeps one error for one situation.
    let (whole, rest) = usize::try_from(needed)
        .ok()
        .and_then(|needed| buffer.split_at_mut_checked(needed))
        .ok_or(too_short)?;
    let (header_slot, payload_slot) = whole
        .split_at_mut_checked(encoded_header.len())
        .ok_or(too_short)?;

    header_slot.copy_from_slice(encoded_header);
    payload.encode_payload(payload_slot)?;

    Ok(rest)
}

/// Value that writes itself as a whole box, header and all
///
/// [`BoxEncode`] writes the payload, and [`BoxDefinition`] names the box type;
/// between them the header is settled, since its remaining field is the total
/// the payload length implies. A value with both therefore knows its whole wire
/// form already, and this trait is that combination — every such value has it,
/// and nothing has to be written to opt in.
///
/// [`AnyBox`](crate::AnyBox) carries its box type as a value rather than a
/// constant, so it cannot implement [`BoxDefinition`] and does not have this
/// trait. It offers the same two operations as inherent methods.
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxDefinition, BoxEncode, BoxType, BoxWrite, Error};
///
/// // A box whose payload is one 32-bit sequence number
/// struct SequenceNumberBox {
///     sequence_number: u32,
/// }
///
/// impl BoxDefinition for SequenceNumberBox {
///     const BOX_TYPE: BoxType = BoxType::compact(*b"sqnc");
/// }
///
/// impl BoxEncode for SequenceNumberBox {
///     fn payload_len(&self) -> u64 {
///         4
///     }
///
///     fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), Error> {
///         let mismatch = Error::buffer_length_mismatch(
///             4,
///             u64::try_from(buffer.len()).unwrap_or(u64::MAX),
///         );
///         let field = buffer.first_chunk_mut::<4>().ok_or(mismatch)?;
///         *field = self.sequence_number.to_be_bytes();
///
///         Ok(())
///     }
/// }
///
/// // The whole box is the eight-byte header and the payload after it
/// let sequence = SequenceNumberBox { sequence_number: 7 };
/// assert_eq!(sequence.encoded_len(), 12);
///
/// let mut buffer = vec![0; 12];
/// assert!(sequence.encode(&mut buffer).unwrap().is_empty());
/// assert_eq!(buffer, b"\0\0\0\x0csqnc\0\0\0\x07");
///
/// // A container writes its children by threading one buffer through them
/// let mut buffer = vec![0; 24];
/// let rest = sequence.encode(&mut buffer).unwrap();
/// assert!(sequence.encode(rest).unwrap().is_empty());
/// assert_eq!(buffer, b"\0\0\0\x0csqnc\0\0\0\x07\0\0\0\x0csqnc\0\0\0\x07");
///
/// // A buffer the box does not fit in is refused
/// assert_eq!(
///     sequence.encode(&mut [0; 11]),
///     Err(Error::truncated_buffer(12, 11))
/// );
/// ```
pub trait BoxWrite: BoxDefinition + BoxEncode {
    /// Returns the length of the whole box, header included
    ///
    /// Saturates at `u64::MAX` where the header and payload together overrun
    /// it, a total no buffer on any target can hold anyway.
    #[must_use]
    fn encoded_len(&self) -> u64 {
        encoded_len_of(Self::BOX_TYPE, self)
    }

    /// Writes the whole box into the front of `buffer` and returns what is left
    ///
    /// `buffer` is at least [`encoded_len`](Self::encoded_len) bytes long. Room
    /// to spare is what the returned remainder is for: a container writes its
    /// children by passing that remainder to the next one in turn.
    ///
    /// An `Err` may leave `buffer` written to in part — the header goes down
    /// before the payload does, so a payload that fails partway leaves a header
    /// that reads as whole in front of bytes that are not. A caller that writes
    /// out what it has on failure would emit that.
    ///
    /// # Errors
    ///
    /// * [`TruncatedBuffer`](crate::ErrorKind::TruncatedBuffer): `buffer` is shorter
    ///   than [`encoded_len`](Self::encoded_len).
    /// * What [`encode_payload`](BoxEncode::encode_payload) reports, for the
    ///   payload written after the header.
    fn encode<'buffer>(&self, buffer: &'buffer mut [u8]) -> Result<&'buffer mut [u8], Error> {
        encode_into(Self::BOX_TYPE, self, buffer)
    }
}

impl<Payload: BoxDefinition + BoxEncode> BoxWrite for Payload {}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::BoxWrite;
    use crate::codec::box_definition::BoxDefinition;
    use crate::codec::box_encode::BoxEncode;
    use crate::data_types::uuid::Uuid;
    use crate::error::{Error, byte_count};
    use crate::framing::box_type::BoxType;

    /// Box whose payload is as long as it is told to be, and is written as zeros
    struct PaddingBox {
        payload_len: u64,
    }

    impl BoxDefinition for PaddingBox {
        const BOX_TYPE: BoxType = BoxType::compact(*b"free");
    }

    impl BoxEncode for PaddingBox {
        fn payload_len(&self) -> u64 {
            self.payload_len
        }

        fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), Error> {
            let actual = byte_count(buffer.len());
            if actual != self.payload_len {
                return Err(Error::buffer_length_mismatch(self.payload_len, actual));
            }

            buffer.fill(0);

            Ok(())
        }
    }

    /// Box named by a user type, which adds sixteen bytes to its header
    struct VendorBox;

    impl BoxDefinition for VendorBox {
        const BOX_TYPE: BoxType = BoxType::Extended(Uuid::new([0xab; 16]));
    }

    impl BoxEncode for VendorBox {
        fn payload_len(&self) -> u64 {
            0
        }

        fn encode_payload(&self, _buffer: &mut [u8]) -> Result<(), Error> {
            Ok(())
        }
    }

    #[test]
    fn a_payload_too_long_for_the_size_field_is_counted_with_a_largesize_header() {
        let padding = PaddingBox {
            payload_len: u64::from(u32::MAX),
        };

        assert_eq!(padding.encoded_len(), u64::from(u32::MAX) + 16);
    }

    #[test]
    fn a_user_type_is_counted_in_the_length_of_the_box() {
        assert_eq!(VendorBox.encoded_len(), 24);
    }

    #[test]
    fn a_user_type_box_writes_its_uuid_into_the_header() {
        let mut buffer = vec![0; 24];

        VendorBox.encode(&mut buffer).unwrap();

        assert_eq!(buffer, [b"\0\0\0\x18uuid".as_slice(), &[0xab; 16]].concat());
    }

    #[test]
    fn a_buffer_one_byte_short_is_refused() {
        let padding = PaddingBox { payload_len: 4 };

        assert_eq!(
            padding.encode(&mut [0; 11]),
            Err(Error::truncated_buffer(12, 11))
        );
    }

    #[test]
    fn a_payload_whose_box_overruns_the_address_space_is_refused_as_a_short_buffer() {
        let padding = PaddingBox {
            payload_len: u64::MAX,
        };

        assert_eq!(
            padding.encode(&mut [0; 16]),
            Err(Error::truncated_buffer(u64::MAX, 16))
        );
    }
}
