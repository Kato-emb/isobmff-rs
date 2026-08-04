//! [`BoxDecode`] and [`DecodeError`], the box payload of ISO/IEC 14496-12 §4.2 read into a value

use core::error;
use core::fmt;

/// Value that the payload of a box decodes into
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxDecode, DecodeError};
///
/// // A box whose payload is one 32-bit sequence number
/// #[derive(PartialEq, Debug)]
/// struct SequenceNumberBox {
///     sequence_number: u32,
/// }
///
/// impl BoxDecode for SequenceNumberBox {
///     fn decode_payload(payload: &[u8]) -> Result<Self, DecodeError> {
///         let (field, rest) =
///             payload
///                 .split_first_chunk::<4>()
///                 .ok_or(DecodeError::TruncatedPayload {
///                     needed: 4,
///                     available: u64::try_from(payload.len()).unwrap_or(u64::MAX),
///                 })?;
///
///         if !rest.is_empty() {
///             return Err(DecodeError::TrailingBytes {
///                 remaining: u64::try_from(rest.len()).unwrap_or(u64::MAX),
///             });
///         }
///
///         Ok(Self {
///             sequence_number: u32::from_be_bytes(*field),
///         })
///     }
/// }
///
/// // The payload arrives whole, the header already consumed by the caller
/// assert_eq!(
///     SequenceNumberBox::decode_payload(b"\0\0\0\x07"),
///     Ok(SequenceNumberBox { sequence_number: 7 })
/// );
///
/// // A payload ending inside the field says how far it had to reach
/// assert_eq!(
///     SequenceNumberBox::decode_payload(b"\0\0\0"),
///     Err(DecodeError::TruncatedPayload {
///         needed: 4,
///         available: 3
///     })
/// );
///
/// // Bytes past the field are an error, not a remainder to skip over
/// assert_eq!(
///     SequenceNumberBox::decode_payload(b"\0\0\0\x07!"),
///     Err(DecodeError::TrailingBytes { remaining: 1 })
/// );
/// ```
pub trait BoxDecode: Sized {
    /// Decodes the payload of one box into a value
    ///
    /// `payload` is that payload whole and nothing besides: framing is settled
    /// before the call and the header is gone, as
    /// [`RawBox::payload`](crate::RawBox::payload) leaves it. Routing a box type
    /// to the implementation that reads it is the caller's part as well.
    ///
    /// Reading is strict. Every byte of `payload` belongs to a field of `Self`,
    /// and bytes the fields do not claim are
    /// [`TrailingBytes`](DecodeError::TrailingBytes) rather than a remainder to
    /// pass over.
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](DecodeError::TruncatedPayload): `payload` ends
    ///   inside a field.
    /// * [`TrailingBytes`](DecodeError::TrailingBytes): `payload` holds bytes
    ///   past the fields of `Self`.
    fn decode_payload(payload: &[u8]) -> Result<Self, DecodeError>;
}

/// Reason a payload does not read as the box it was framed as
///
/// Framing is settled before a payload reaches [`BoxDecode::decode_payload`],
/// so a frame that does not hold is [`BoxHeaderError`](crate::BoxHeaderError),
/// the error of the layer before this one.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DecodeError {
    /// Payload ends inside a field
    TruncatedPayload {
        /// Bytes the fields read so far require
        needed: u64,
        /// Bytes the payload offered
        available: u64,
    },
    /// Payload holds bytes past the fields of the box
    TrailingBytes {
        /// Bytes left over once every field was read
        remaining: u64,
    },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::TruncatedPayload { needed, available } => write!(
                formatter,
                "box payload of {needed} bytes cut short by an input of {available}"
            ),
            Self::TrailingBytes { remaining } => write!(
                formatter,
                "box payload leaves {remaining} bytes past the fields it holds"
            ),
        }
    }
}

impl error::Error for DecodeError {}

#[cfg(test)]
mod tests {
    use alloc::string::ToString as _;

    use super::DecodeError;

    #[test]
    fn display_of_a_truncated_payload_names_both_lengths() {
        let error = DecodeError::TruncatedPayload {
            needed: 16,
            available: 12,
        };

        assert_eq!(
            error.to_string(),
            "box payload of 16 bytes cut short by an input of 12"
        );
    }

    #[test]
    fn display_of_trailing_bytes_names_how_many_are_left() {
        let error = DecodeError::TrailingBytes { remaining: 4 };

        assert_eq!(
            error.to_string(),
            "box payload leaves 4 bytes past the fields it holds"
        );
    }
}
