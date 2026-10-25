//! [`BoxEncode`], a box written from a value

use crate::codec::box_definition::BoxDefinition;
use crate::codec::field::FieldWriter;
use crate::error::Error;
use crate::framing::box_header::BoxHeader;

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
/// let mut buffer = [0; 4];
///
/// assert_eq!(sequence.encode_payload(&mut buffer), Ok(()));
/// assert_eq!(buffer, *b"\0\0\0\x07");
///
/// // A buffer with room to spare is refused as a short one is
/// assert_eq!(
///     sequence.encode_payload(&mut [0; 8]),
///     Err(Error::buffer_length_mismatch(4, 8))
/// );
/// ```
pub trait BoxEncode {
    /// Returns the length of the payload that [`encode_payload`](Self::encode_payload) writes
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
    /// it. A field of bytes writes whole with
    /// [`write_slice`](FieldWriter::write_slice), and a run of child boxes
    /// takes what is left with [`take_remainder`](FieldWriter::take_remainder).
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
    /// * What else the box makes of the value it holds, such as a value the
    ///   specification does not allow.
    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error>;

    /// Writes the payload of the value into `buffer`
    ///
    /// `buffer` is held to [`payload_len`](Self::payload_len) — no shorter and
    /// no longer — before a byte is written, and to being claimed whole once the
    /// fields are written.
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
        let offered = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        if offered != declared {
            return Err(Error::buffer_length_mismatch(declared, offered));
        }

        let mut writer = FieldWriter::new(buffer);
        self.encode_fields(&mut writer)?;

        writer.finish()
    }

    /// Returns the length of the whole box, header included
    ///
    /// [`BoxDefinition`] names the box type, and the header that introduces
    /// [`payload_len`](Self::payload_len) bytes under it is the shortest one
    /// [`BoxHeader::with_payload_len`] builds.
    ///
    /// Saturates at `u64::MAX` where the header and payload together overrun it.
    #[must_use]
    fn encoded_len(&self) -> u64
    where
        Self: BoxDefinition + Sized,
    {
        let payload_len = self.payload_len();

        BoxHeader::with_payload_len(Self::BOX_TYPE, payload_len).map_or(u64::MAX, |header| {
            header.encoded_len().saturating_add(payload_len)
        })
    }

    /// Writes the whole box into the front of `buffer` and returns what is left
    ///
    /// The header goes down under [`BOX_TYPE`](BoxDefinition::BOX_TYPE) and the
    /// total that [`payload_len`](Self::payload_len) implies, and the payload
    /// follows as [`encode_payload`](Self::encode_payload) writes it. It is the
    /// mirror of [`BoxDecode::decode`](crate::BoxDecode::decode), which reads
    /// the same box back.
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
    /// * What [`BoxHeader::encode`] reports for the header, and what
    ///   [`encode_payload`](Self::encode_payload) reports for the payload.
    ///
    /// # Examples
    ///
    /// ```
    /// use isobmff_core::{BoxDefinition, BoxEncode, BoxType, Error, FieldWriter};
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
    ///     fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
    ///         writer.write_u32(self.sequence_number)
    ///     }
    /// }
    ///
    /// // The whole box is the eight-byte header and the payload after it
    /// let sequence = SequenceNumberBox { sequence_number: 7 };
    /// assert_eq!(sequence.encoded_len(), 12);
    ///
    /// // A container writes its children by threading one buffer through them
    /// let mut buffer = [0; 24];
    /// let rest = sequence.encode(&mut buffer).unwrap();
    /// assert!(sequence.encode(rest).unwrap().is_empty());
    /// assert_eq!(&buffer, b"\0\0\0\x0csqnc\0\0\0\x07\0\0\0\x0csqnc\0\0\0\x07");
    ///
    /// // A buffer the box does not fit in is refused
    /// assert_eq!(
    ///     sequence.encode(&mut [0; 11]),
    ///     Err(Error::truncated_buffer(12, 11))
    /// );
    /// ```
    fn encode<'buffer>(&self, buffer: &'buffer mut [u8]) -> Result<&'buffer mut [u8], Error>
    where
        Self: BoxDefinition + Sized,
    {
        let needed = self.encoded_len();
        let available = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        let too_short = || Error::truncated_buffer(needed, available);

        let header = BoxHeader::with_payload_len(Self::BOX_TYPE, self.payload_len())
            .ok_or_else(too_short)?;
        // Why not report a total beyond `usize` as its own error: such a total
        // exceeds any `buffer.len()` on the same target, so it is a short buffer
        // by another name and folding it in keeps one error for one situation.
        let (whole, rest) = usize::try_from(needed)
            .ok()
            .and_then(|needed| buffer.split_at_mut_checked(needed))
            .ok_or_else(too_short)?;

        self.encode_payload(header.encode(whole)?)?;

        Ok(rest)
    }
}

#[cfg(test)]
mod tests {
    use super::BoxEncode;
    use crate::codec::box_definition::BoxDefinition;
    use crate::codec::field::FieldWriter;
    use crate::error::Error;
    use crate::framing::box_type::BoxType;

    /// Box whose payload is one 32-bit sequence number, declaring a payload length of its own
    struct SequenceNumberBox {
        sequence_number: u32,
        declared_payload_len: u64,
    }

    impl BoxDefinition for SequenceNumberBox {
        const BOX_TYPE: BoxType = BoxType::compact(*b"sqnc");
    }

    impl BoxEncode for SequenceNumberBox {
        fn payload_len(&self) -> u64 {
            self.declared_payload_len
        }

        fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
            writer.write_u32(self.sequence_number)
        }
    }

    #[test]
    fn a_payload_declaring_more_than_its_fields_write_is_refused() {
        let sequence = SequenceNumberBox {
            sequence_number: 7,
            declared_payload_len: 5,
        };

        assert_eq!(
            sequence.encode(&mut [0; 13]),
            Err(Error::trailing_buffer(4, 5))
        );
    }

    #[test]
    fn a_payload_declaring_less_than_its_fields_write_is_refused() {
        let sequence = SequenceNumberBox {
            sequence_number: 7,
            declared_payload_len: 3,
        };

        assert_eq!(
            sequence.encode(&mut [0; 11]),
            Err(Error::truncated_buffer(4, 3))
        );
    }

    #[test]
    fn an_extended_box_type_writes_its_uuid_after_the_boxtype_field() {
        struct VendorBox;

        impl BoxDefinition for VendorBox {
            const BOX_TYPE: BoxType = BoxType::extended([0xab; 16]);
        }

        impl BoxEncode for VendorBox {
            fn payload_len(&self) -> u64 {
                1
            }

            fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
                writer.write_bytes(&[0x01])
            }
        }

        let mut buffer = [0; 25];

        assert_eq!(VendorBox.encode(&mut buffer).map(|rest| rest.len()), Ok(0));
        assert_eq!(
            buffer,
            *b"\0\0\0\x19uuid\xab\xab\xab\xab\xab\xab\xab\xab\xab\xab\xab\xab\xab\xab\xab\xab\x01"
        );
    }
}
