//! [`BoxEncode`] and [`EncodeError`], the box payload of ISO/IEC 14496-12 §4.2 written from a value

use core::error;
use core::fmt;

use crate::field::FieldWriteError;

/// Value that writes itself as the payload of a box
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxEncode, EncodeError};
///
/// // A box whose payload is one 32-bit sequence number
/// struct SequenceNumberBox {
///     sequence_number: u32,
/// }
///
/// impl BoxEncode for SequenceNumberBox {
///     fn payload_len(&self) -> u64 {
///         4
///     }
///
///     fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), EncodeError> {
///         let mismatch = EncodeError::BufferLengthMismatch {
///             expected: self.payload_len(),
///             actual: u64::try_from(buffer.len()).unwrap_or(u64::MAX),
///         };
///         if buffer.len() != 4 {
///             return Err(mismatch);
///         }
///
///         let field = buffer.first_chunk_mut::<4>().ok_or(mismatch)?;
///         *field = self.sequence_number.to_be_bytes();
///
///         Ok(())
///     }
/// }
///
/// // The buffer is sized from what the value declares
/// let sequence = SequenceNumberBox { sequence_number: 7 };
/// let mut buffer = vec![0; usize::try_from(sequence.payload_len()).unwrap()];
///
/// assert_eq!(sequence.encode_payload(&mut buffer), Ok(()));
/// assert_eq!(buffer, b"\0\0\0\x07".as_slice());
///
/// // A buffer with room to spare is refused as a short one is
/// assert_eq!(
///     sequence.encode_payload(&mut [0; 8]),
///     Err(EncodeError::BufferLengthMismatch {
///         expected: 4,
///         actual: 8
///     })
/// );
/// ```
pub trait BoxEncode {
    /// Returns the length of the payload that
    /// [`encode_payload`](Self::encode_payload) writes
    ///
    /// The value declares this before a byte is written, so a caller can size
    /// the buffer and settle the total the header declares. Nothing checks the
    /// two against each other; that they agree is what an implementation
    /// promises here.
    #[must_use]
    fn payload_len(&self) -> u64;

    /// Writes the payload of the value into `buffer`
    ///
    /// `buffer` is exactly [`payload_len`](Self::payload_len) bytes long, no
    /// shorter and no longer.
    ///
    /// An implementation that has matched the length once may report the
    /// failures that can no longer happen — a chunk that will not split off a
    /// buffer already known to be long enough — as the same mismatch.
    ///
    /// # Errors
    ///
    /// * [`BufferLengthMismatch`](EncodeError::BufferLengthMismatch): `buffer`
    ///   is not [`payload_len`](Self::payload_len) bytes long.
    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), EncodeError>;
}

/// Reason a value does not write as the payload of its box
///
/// The two say different things about `buffer`.
/// [`BufferLengthMismatch`](Self::BufferLengthMismatch) comes from a payload,
/// which is handed a buffer of exactly its own length and refuses any other.
/// [`BufferTooShort`](Self::BufferTooShort) comes from a value written into a
/// buffer it shares with what follows it — a whole box, or one field among
/// several — which takes what it needs off the front and leaves the rest. Room
/// to spare is expected there, and only a shortfall is an error.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum EncodeError {
    /// Buffer offered is not the length the payload declared
    BufferLengthMismatch {
        /// Bytes the payload occupies, as
        /// [`payload_len`](BoxEncode::payload_len) declares
        expected: u64,
        /// Bytes the buffer offered
        actual: u64,
    },
    /// Buffer offered is shorter than the value needs
    BufferTooShort {
        /// Bytes the value occupies on the wire
        needed: u64,
        /// Bytes the buffer offered
        available: u64,
    },
    /// Fields of the box do not write into the buffer of its payload
    Field(FieldWriteError),
}

impl From<FieldWriteError> for EncodeError {
    fn from(error: FieldWriteError) -> Self {
        Self::Field(error)
    }
}

impl fmt::Display for EncodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::BufferLengthMismatch { expected, actual } => write!(
                formatter,
                "box payload of {expected} bytes needs a buffer of that length, not {actual}"
            ),
            Self::BufferTooShort { needed, available } => write!(
                formatter,
                "value of {needed} bytes needs a buffer at least that long, not {available}"
            ),
            Self::Field(_) => {
                formatter.write_str("box payload does not write as the fields it holds")
            }
        }
    }
}

impl error::Error for EncodeError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match *self {
            Self::BufferLengthMismatch { .. } | Self::BufferTooShort { .. } => None,
            Self::Field(ref error) => Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString as _;

    use super::EncodeError;

    #[test]
    fn display_of_a_buffer_length_mismatch_names_both_lengths() {
        let error = EncodeError::BufferLengthMismatch {
            expected: 16,
            actual: 12,
        };

        assert_eq!(
            error.to_string(),
            "box payload of 16 bytes needs a buffer of that length, not 12"
        );
    }

    #[test]
    fn display_of_a_buffer_too_short_names_both_lengths() {
        let error = EncodeError::BufferTooShort {
            needed: 24,
            available: 16,
        };

        assert_eq!(
            error.to_string(),
            "value of 24 bytes needs a buffer at least that long, not 16"
        );
    }
}
