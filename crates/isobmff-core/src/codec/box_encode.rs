//! [`BoxEncode`], the box payload of ISO/IEC 14496-12 §4.2 written from a value

use crate::codec::field::FieldWriter;
use crate::error::{Error, byte_count};

/// Value that writes itself as the payload of a box
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxEncode, Error, FieldWriter};
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
///     fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
///         writer.write_u32(self.sequence_number)
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
    /// the buffer and settle the total the header declares.
    /// [`encode_payload`](Self::encode_payload) is where the declaration is held
    /// to what the buffer and the fields turn out to be.
    #[must_use]
    fn payload_len(&self) -> u64;

    /// Writes the fields of the value onto the front of the payload of one box
    ///
    /// The mirror of [`BoxDecode::decode_fields`](crate::BoxDecode::decode_fields):
    /// the Syntax subclause of the box lays out the fields, and an
    /// implementation writes each onto `writer` in the order the box declares
    /// it. A field that runs to the end of the payload takes what is left with
    /// [`take_remainder`](FieldWriter::take_remainder).
    ///
    /// The buffer behind `writer` is exactly [`payload_len`](Self::payload_len)
    /// bytes long, which [`encode_payload`](Self::encode_payload) settles
    /// around this call.
    ///
    /// # Errors
    ///
    /// * [`TruncatedBuffer`](crate::ErrorKind::TruncatedBuffer): the fields reach
    ///   past the buffer, which is [`payload_len`](Self::payload_len) declaring
    ///   less than the fields write.
    /// * [`OutOfRange`](crate::ErrorKind::OutOfRange): a value the box holds does
    ///   not fit the width the wire gives its field.
    /// * What else the box makes of the value it holds, such as the failures a
    ///   container brings.
    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error>;

    /// Writes the payload of the value into `buffer`
    ///
    /// `buffer` is held to [`payload_len`](Self::payload_len) — no shorter and
    /// no longer — before a byte is written, and to being claimed whole once the
    /// fields are written. This is what the method holds
    /// [`encode_fields`](Self::encode_fields) to, so an implementation states
    /// the fields and leaves this one as it stands.
    ///
    /// Claimed is not the same as written: a field running to the end of the
    /// payload takes the bytes it does not write into, which keep whatever the
    /// buffer held.
    ///
    /// # Errors
    ///
    /// * [`BufferLengthMismatch`](crate::ErrorKind::BufferLengthMismatch):
    ///   `buffer` is not [`payload_len`](Self::payload_len) bytes long, which is
    ///   the caller sizing it from something else.
    /// * [`TrailingBuffer`](crate::ErrorKind::TrailingBuffer): the fields left
    ///   bytes of `buffer` unclaimed, which is
    ///   [`payload_len`](Self::payload_len) declaring more than the fields
    ///   claim.
    /// * What [`encode_fields`](Self::encode_fields) reports for the fields
    ///   themselves.
    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), Error> {
        let declared = self.payload_len();
        let offered = byte_count(buffer.len());
        if offered != declared {
            return Err(Error::buffer_length_mismatch(declared, offered));
        }

        let mut writer = FieldWriter::new(buffer);
        self.encode_fields(&mut writer)?;

        writer.finish()
    }
}
