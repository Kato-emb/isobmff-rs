//! [`BoxEncode`], the box payload of ISO/IEC 14496-12 §4.2 written from a value

use crate::error::Error;

/// Value that writes itself as the payload of a box
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxEncode, Error};
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
///     fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), Error> {
///         let mismatch = Error::buffer_length_mismatch(
///             self.payload_len(),
///             u64::try_from(buffer.len()).unwrap_or(u64::MAX),
///         );
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
///     Err(Error::buffer_length_mismatch(4, 8))
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
    /// * [`BufferLengthMismatch`](crate::ErrorKind::BufferLengthMismatch):
    ///   `buffer` is not [`payload_len`](Self::payload_len) bytes long.
    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), Error>;
}
