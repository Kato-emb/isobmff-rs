//! [`FieldReader`] and [`FieldWriter`], the fields a box payload of ISO/IEC 14496-12 §4.2 is made of

use core::error;
use core::fmt;
use core::mem;

/// Width a box settles for a field it carries at more than one size
///
/// A full box that carries a value both ways says which it used in its
/// version, and every field that widens with the version takes the same width.
/// Choosing it is the box's part; this is what the choice comes to, and
/// [`read_unsigned`](FieldReader::read_unsigned) and
/// [`write_unsigned`](FieldWriter::write_unsigned) are where it is spent.
///
/// # Examples
///
/// ```
/// use isobmff_core::{FieldReader, FieldWidth};
///
/// // The width a box carrying its times in 64 bits settles on
/// let width = FieldWidth::Extended;
///
/// // The field occupies those eight bytes and reads as the integer they spell
/// let mut reader = FieldReader::new(b"\0\0\0\0\0\0\0\x07");
/// assert_eq!(reader.read_unsigned(width), Ok(7));
/// assert_eq!(reader.finish(), Ok(()));
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FieldWidth {
    /// Field of four bytes
    Compact,
    /// Field of eight bytes
    Extended,
}

/// Cursor reading the fields of a box payload off its front
///
/// A payload is a run of fields laid end to end, and reading one is taking the
/// bytes of every field off the front in the order the box declares them. The
/// cursor carries how far the reading has reached, so a field running past the
/// end of the payload names both what the fields required up to it and what
/// the payload offered.
///
/// A box whose payload is fixed calls [`finish`](Self::finish) once its fields
/// are read, which refuses a payload with bytes to spare. One whose last field
/// runs to the end of the payload takes what is left with
/// [`remainder`](Self::remainder) instead.
///
/// # Examples
///
/// ```
/// use isobmff_core::{FieldReadError, FieldReader};
///
/// // A payload of a 32-bit identifier and a 16-bit count
/// let mut reader = FieldReader::new(b"\0\0\0\x07\0\x02");
/// assert_eq!(reader.read_u32().unwrap(), 7);
/// assert_eq!(reader.read_u16().unwrap(), 2);
///
/// // Every byte of the payload belongs to a field of the box
/// assert_eq!(reader.finish(), Ok(()));
///
/// // A payload ending inside a field says how far the fields had to reach
/// let mut truncated = FieldReader::new(b"\0\0\0");
/// assert_eq!(
///     truncated.read_u32(),
///     Err(FieldReadError::UnexpectedEof {
///         needed: 4,
///         available: 3
///     })
/// );
/// ```
#[derive(Debug)]
pub struct FieldReader<'payload> {
    rest: &'payload [u8],
    consumed: u64,
}

impl<'payload> FieldReader<'payload> {
    /// Creates a cursor over the payload of one box
    #[must_use]
    pub fn new(payload: &'payload [u8]) -> Self {
        Self {
            rest: payload,
            consumed: 0,
        }
    }

    /// Reads the next field, which occupies `N` bytes
    ///
    /// # Errors
    ///
    /// * [`UnexpectedEof`](FieldReadError::UnexpectedEof): the payload ends
    ///   inside the field.
    pub fn read_bytes<const N: usize>(&mut self) -> Result<&'payload [u8; N], FieldReadError> {
        let needed = self.consumed.saturating_add(byte_count(N));
        let rest = self.rest;
        let (field, tail) =
            rest.split_first_chunk::<N>()
                .ok_or_else(|| FieldReadError::UnexpectedEof {
                    needed,
                    available: self.consumed.saturating_add(byte_count(rest.len())),
                })?;

        self.rest = tail;
        self.consumed = needed;

        Ok(field)
    }

    /// Reads the next field as a 16-bit unsigned integer
    ///
    /// # Errors
    ///
    /// * [`UnexpectedEof`](FieldReadError::UnexpectedEof): the payload ends
    ///   inside the field.
    pub fn read_u16(&mut self) -> Result<u16, FieldReadError> {
        Ok(u16::from_be_bytes(*self.read_bytes::<2>()?))
    }

    /// Reads the next field as a 16-bit signed integer
    ///
    /// # Errors
    ///
    /// * [`UnexpectedEof`](FieldReadError::UnexpectedEof): the payload ends
    ///   inside the field.
    pub fn read_i16(&mut self) -> Result<i16, FieldReadError> {
        Ok(i16::from_be_bytes(*self.read_bytes::<2>()?))
    }

    /// Reads the next field as a 32-bit unsigned integer
    ///
    /// # Errors
    ///
    /// * [`UnexpectedEof`](FieldReadError::UnexpectedEof): the payload ends
    ///   inside the field.
    pub fn read_u32(&mut self) -> Result<u32, FieldReadError> {
        Ok(u32::from_be_bytes(*self.read_bytes::<4>()?))
    }

    /// Reads the next field as a 32-bit signed integer
    ///
    /// # Errors
    ///
    /// * [`UnexpectedEof`](FieldReadError::UnexpectedEof): the payload ends
    ///   inside the field.
    pub fn read_i32(&mut self) -> Result<i32, FieldReadError> {
        Ok(i32::from_be_bytes(*self.read_bytes::<4>()?))
    }

    /// Reads the next field as a 64-bit unsigned integer
    ///
    /// # Errors
    ///
    /// * [`UnexpectedEof`](FieldReadError::UnexpectedEof): the payload ends
    ///   inside the field.
    pub fn read_u64(&mut self) -> Result<u64, FieldReadError> {
        Ok(u64::from_be_bytes(*self.read_bytes::<8>()?))
    }

    /// Reads the next field as an unsigned integer of the width the box settled
    ///
    /// # Errors
    ///
    /// * [`UnexpectedEof`](FieldReadError::UnexpectedEof): the payload ends
    ///   inside the field.
    pub fn read_unsigned(&mut self, width: FieldWidth) -> Result<u64, FieldReadError> {
        match width {
            FieldWidth::Compact => Ok(u64::from(self.read_u32()?)),
            FieldWidth::Extended => self.read_u64(),
        }
    }

    /// Returns the bytes of the payload no field has taken
    ///
    /// A box reads its last field out of these when the field runs to the end
    /// of the payload, which is how a variable-length field is bounded.
    #[must_use]
    pub fn remainder(&self) -> &'payload [u8] {
        self.rest
    }

    /// Reports the payload as read whole, which every field of a fixed box has claimed
    ///
    /// # Errors
    ///
    /// * [`TrailingBytes`](FieldReadError::TrailingBytes): the payload holds
    ///   bytes past the fields of the box.
    pub fn finish(self) -> Result<(), FieldReadError> {
        if self.rest.is_empty() {
            return Ok(());
        }

        Err(FieldReadError::TrailingBytes {
            remaining: byte_count(self.rest.len()),
        })
    }
}

/// Cursor writing the fields of a box payload onto its front
///
/// The cursor is the mirror of [`FieldReader`]: it takes the bytes of every
/// field off the front of a buffer in the order the box declares them, and
/// carries how far the writing has reached.
///
/// The buffer is the one [`payload_len`](crate::BoxEncode::payload_len)
/// declared, so a box that fills it exactly has written the payload it
/// promised. [`finish`](Self::finish) is where the two are held against each
/// other: a buffer with bytes to spare means the declared length is longer
/// than the fields, as running out of buffer mid-field means it is shorter.
///
/// # Examples
///
/// ```
/// use isobmff_core::{FieldWriteError, FieldWriter};
///
/// // A payload of a 32-bit identifier and a 16-bit count
/// let mut buffer = [0; 6];
/// let mut writer = FieldWriter::new(&mut buffer);
/// writer.write_u32(7).unwrap();
/// writer.write_u16(2).unwrap();
///
/// // The fields have filled the buffer the payload declared
/// assert_eq!(writer.finish(), Ok(()));
/// assert_eq!(buffer, *b"\0\0\0\x07\0\x02");
///
/// // A buffer that runs out mid-field says what the fields required
/// let mut narrow = [0; 3];
/// assert_eq!(
///     FieldWriter::new(&mut narrow).write_u32(7),
///     Err(FieldWriteError::UnexpectedEof {
///         needed: 4,
///         available: 3
///     })
/// );
/// ```
#[derive(Debug)]
pub struct FieldWriter<'buffer> {
    rest: &'buffer mut [u8],
    written: u64,
}

impl<'buffer> FieldWriter<'buffer> {
    /// Creates a cursor over the buffer the payload of one box occupies
    #[must_use]
    pub fn new(buffer: &'buffer mut [u8]) -> Self {
        Self {
            rest: buffer,
            written: 0,
        }
    }

    /// Writes the next field, which occupies `N` bytes
    ///
    /// # Errors
    ///
    /// * [`UnexpectedEof`](FieldWriteError::UnexpectedEof): the buffer ends
    ///   inside the field.
    pub fn write_bytes<const N: usize>(&mut self, bytes: &[u8; N]) -> Result<(), FieldWriteError> {
        let needed = self.written.saturating_add(byte_count(N));
        let available = self.written.saturating_add(byte_count(self.rest.len()));
        // Why not leaving the buffer in place: the field does not fit, so the
        // box abandons the writer, and what is held back would be a buffer no
        // field can be written into anyway.
        let Some((field, tail)) = mem::take(&mut self.rest).split_first_chunk_mut::<N>() else {
            return Err(FieldWriteError::UnexpectedEof { needed, available });
        };

        *field = *bytes;
        self.rest = tail;
        self.written = needed;

        Ok(())
    }

    /// Writes the next field as a 16-bit unsigned integer
    ///
    /// # Errors
    ///
    /// * [`UnexpectedEof`](FieldWriteError::UnexpectedEof): the buffer ends
    ///   inside the field.
    pub fn write_u16(&mut self, value: u16) -> Result<(), FieldWriteError> {
        self.write_bytes(&value.to_be_bytes())
    }

    /// Writes the next field as a 16-bit signed integer
    ///
    /// # Errors
    ///
    /// * [`UnexpectedEof`](FieldWriteError::UnexpectedEof): the buffer ends
    ///   inside the field.
    pub fn write_i16(&mut self, value: i16) -> Result<(), FieldWriteError> {
        self.write_bytes(&value.to_be_bytes())
    }

    /// Writes the next field as a 32-bit unsigned integer
    ///
    /// # Errors
    ///
    /// * [`UnexpectedEof`](FieldWriteError::UnexpectedEof): the buffer ends
    ///   inside the field.
    pub fn write_u32(&mut self, value: u32) -> Result<(), FieldWriteError> {
        self.write_bytes(&value.to_be_bytes())
    }

    /// Writes the next field as a 32-bit signed integer
    ///
    /// # Errors
    ///
    /// * [`UnexpectedEof`](FieldWriteError::UnexpectedEof): the buffer ends
    ///   inside the field.
    pub fn write_i32(&mut self, value: i32) -> Result<(), FieldWriteError> {
        self.write_bytes(&value.to_be_bytes())
    }

    /// Writes the next field as a 64-bit unsigned integer
    ///
    /// # Errors
    ///
    /// * [`UnexpectedEof`](FieldWriteError::UnexpectedEof): the buffer ends
    ///   inside the field.
    pub fn write_u64(&mut self, value: u64) -> Result<(), FieldWriteError> {
        self.write_bytes(&value.to_be_bytes())
    }

    /// Writes the next field as an unsigned integer of the width the box settled
    ///
    /// # Errors
    ///
    /// * [`UnexpectedEof`](FieldWriteError::UnexpectedEof): the buffer ends
    ///   inside the field.
    /// * [`OutOfRange`](FieldWriteError::OutOfRange): `value` is wider than the
    ///   field, which leaves nothing to write.
    pub fn write_unsigned(&mut self, width: FieldWidth, value: u64) -> Result<(), FieldWriteError> {
        match width {
            FieldWidth::Compact => {
                let narrow = u32::try_from(value)
                    .map_err(|_| FieldWriteError::OutOfRange { value, width })?;

                self.write_u32(narrow)
            }
            FieldWidth::Extended => self.write_u64(value),
        }
    }

    /// Returns the buffer no field has written into
    ///
    /// A box writes its last field into these bytes when the field runs to the
    /// end of the payload, which the cursor no longer tracks once they are
    /// handed over.
    #[must_use]
    pub fn into_remainder(self) -> &'buffer mut [u8] {
        self.rest
    }

    /// Reports the buffer as written whole, which every field of a fixed box has filled
    ///
    /// # Errors
    ///
    /// * [`TrailingBytes`](FieldWriteError::TrailingBytes): the buffer holds
    ///   bytes past the fields the box wrote.
    pub fn finish(self) -> Result<(), FieldWriteError> {
        if self.rest.is_empty() {
            return Ok(());
        }

        Err(FieldWriteError::TrailingBytes {
            remaining: byte_count(self.rest.len()),
        })
    }
}

/// Returns a length of bytes as the count a failure of this module carries
fn byte_count(length: usize) -> u64 {
    // Why not unwrap: a usize above `u64::MAX` needs a 128-bit target to exist,
    // and saturating keeps the panic-free path.
    u64::try_from(length).unwrap_or(u64::MAX)
}

/// Reason the fields of a box do not read off its payload
///
/// A box reports this as [`DecodeError::Field`](crate::DecodeError::Field),
/// which the `?` of a [`decode_payload`](crate::BoxDecode::decode_payload)
/// implementation converts to.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FieldReadError {
    /// Payload ends inside a field
    UnexpectedEof {
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

impl fmt::Display for FieldReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::UnexpectedEof { needed, available } => write!(
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

impl error::Error for FieldReadError {}

/// Reason the fields of a box do not write into the buffer of its payload
///
/// The first two say the same thing about a box: the length its payload
/// declares and the fields it writes do not agree. Which of the two is wrong
/// is the box's to settle — the buffer is the length that was declared. The
/// third is about one field alone, and no buffer would make it write.
///
/// A box reports this as [`EncodeError::Field`](crate::EncodeError::Field),
/// which the `?` of an [`encode_payload`](crate::BoxEncode::encode_payload)
/// implementation converts to.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FieldWriteError {
    /// Buffer ends inside a field
    UnexpectedEof {
        /// Bytes the fields written so far require
        needed: u64,
        /// Bytes the buffer offered
        available: u64,
    },
    /// Buffer holds bytes past the fields the box wrote
    TrailingBytes {
        /// Bytes left over once every field was written
        remaining: u64,
    },
    /// Value is wider than the field it was given to
    OutOfRange {
        /// Value the field was given
        value: u64,
        /// Width of the field that was given it
        width: FieldWidth,
    },
}

impl fmt::Display for FieldWriteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::UnexpectedEof { needed, available } => write!(
                formatter,
                "fields of the box need {needed} bytes but its buffer holds {available}"
            ),
            Self::TrailingBytes { remaining } => write!(
                formatter,
                "buffer holds {remaining} bytes past the fields the box wrote"
            ),
            Self::OutOfRange { value, width } => {
                let bytes = match width {
                    FieldWidth::Compact => 4,
                    FieldWidth::Extended => 8,
                };

                write!(
                    formatter,
                    "value {value} does not fit the {bytes} bytes of the field it was given to"
                )
            }
        }
    }
}

impl error::Error for FieldWriteError {}

#[cfg(test)]
mod tests {
    use alloc::string::ToString as _;

    use super::{FieldReadError, FieldReader, FieldWidth, FieldWriteError, FieldWriter};

    #[test]
    fn fields_are_read_off_the_front_in_the_order_they_are_asked_for() {
        let mut reader = FieldReader::new(b"\x01\x02\x03\x04\x05\x06\x07\x08\xff\xfe");

        assert_eq!(reader.read_u64(), Ok(0x0102_0304_0506_0708));
        assert_eq!(reader.read_i16(), Ok(-2));
        assert_eq!(reader.finish(), Ok(()));
    }

    #[test]
    fn a_field_running_past_the_payload_names_what_the_fields_required() {
        let mut reader = FieldReader::new(b"\x01\x02\x03\x04\x05\x06");

        assert_eq!(reader.read_u32(), Ok(0x0102_0304));
        assert_eq!(
            reader.read_u32(),
            Err(FieldReadError::UnexpectedEof {
                needed: 8,
                available: 6
            })
        );
    }

    #[test]
    fn a_payload_with_bytes_past_its_fields_is_refused() {
        let mut reader = FieldReader::new(b"\x01\x02\x03\x04\x05");

        assert_eq!(reader.read_u32(), Ok(0x0102_0304));
        assert_eq!(
            reader.finish(),
            Err(FieldReadError::TrailingBytes { remaining: 1 })
        );
    }

    #[test]
    fn a_field_of_either_width_reads_as_the_integer_its_bytes_spell() {
        let mut compact = FieldReader::new(b"\0\0\0\x07");
        let mut extended = FieldReader::new(b"\0\0\0\0\0\0\0\x07");

        assert_eq!(compact.read_unsigned(FieldWidth::Compact), Ok(7));
        assert_eq!(extended.read_unsigned(FieldWidth::Extended), Ok(7));
    }

    #[test]
    fn the_bytes_no_field_took_are_what_a_variable_field_reads_from() {
        let mut reader = FieldReader::new(b"\0\x02rest of the payload");

        assert_eq!(reader.read_u16(), Ok(2));
        assert_eq!(reader.remainder(), b"rest of the payload");
    }

    #[test]
    fn fields_are_written_onto_the_front_in_the_order_they_are_given() {
        let mut buffer = [0; 10];
        let mut writer = FieldWriter::new(&mut buffer);

        assert_eq!(writer.write_u64(0x0102_0304_0506_0708), Ok(()));
        assert_eq!(writer.write_i16(-2), Ok(()));
        assert_eq!(writer.finish(), Ok(()));
        assert_eq!(&buffer, b"\x01\x02\x03\x04\x05\x06\x07\x08\xff\xfe");
    }

    #[test]
    fn a_field_running_past_the_buffer_names_what_the_fields_required() {
        let mut buffer = [0; 6];
        let mut writer = FieldWriter::new(&mut buffer);

        assert_eq!(writer.write_u32(0x0102_0304), Ok(()));
        assert_eq!(
            writer.write_u32(0x0506_0708),
            Err(FieldWriteError::UnexpectedEof {
                needed: 8,
                available: 6
            })
        );
    }

    #[test]
    fn a_buffer_with_bytes_past_the_fields_written_is_refused() {
        let mut buffer = [0; 5];
        let mut writer = FieldWriter::new(&mut buffer);

        assert_eq!(writer.write_u32(0x0102_0304), Ok(()));
        assert_eq!(
            writer.finish(),
            Err(FieldWriteError::TrailingBytes { remaining: 1 })
        );
    }

    #[test]
    fn a_field_of_either_width_writes_the_bytes_that_width_names() {
        let mut buffer = [0; 12];
        let mut writer = FieldWriter::new(&mut buffer);

        assert_eq!(writer.write_unsigned(FieldWidth::Compact, 7), Ok(()));
        assert_eq!(writer.write_unsigned(FieldWidth::Extended, 7), Ok(()));
        assert_eq!(writer.finish(), Ok(()));
        assert_eq!(&buffer, b"\0\0\0\x07\0\0\0\0\0\0\0\x07");
    }

    #[test]
    fn a_value_wider_than_its_field_is_refused_before_a_byte_is_written() {
        let mut buffer = [0xff; 4];

        assert_eq!(
            FieldWriter::new(&mut buffer)
                .write_unsigned(FieldWidth::Compact, u64::from(u32::MAX) + 1),
            Err(FieldWriteError::OutOfRange {
                value: 0x1_0000_0000,
                width: FieldWidth::Compact
            })
        );
        assert_eq!(buffer, [0xff; 4]);
    }

    #[test]
    fn the_bytes_no_field_wrote_into_are_what_a_variable_field_writes_to() {
        let mut buffer = [0; 6];
        let mut writer = FieldWriter::new(&mut buffer);

        assert_eq!(writer.write_u16(2), Ok(()));
        writer.into_remainder().copy_from_slice(b"tail");

        assert_eq!(&buffer, b"\0\x02tail");
    }

    #[test]
    fn display_of_a_read_failure_names_both_lengths() {
        let truncated = FieldReadError::UnexpectedEof {
            needed: 16,
            available: 12,
        };
        let trailing = FieldReadError::TrailingBytes { remaining: 4 };

        assert_eq!(
            truncated.to_string(),
            "box payload of 16 bytes cut short by an input of 12"
        );
        assert_eq!(
            trailing.to_string(),
            "box payload leaves 4 bytes past the fields it holds"
        );
    }

    #[test]
    fn display_of_a_write_failure_names_both_lengths() {
        let exhausted = FieldWriteError::UnexpectedEof {
            needed: 16,
            available: 12,
        };
        let trailing = FieldWriteError::TrailingBytes { remaining: 4 };
        let out_of_range = FieldWriteError::OutOfRange {
            value: 0x1_0000_0000,
            width: FieldWidth::Compact,
        };

        assert_eq!(
            exhausted.to_string(),
            "fields of the box need 16 bytes but its buffer holds 12"
        );
        assert_eq!(
            trailing.to_string(),
            "buffer holds 4 bytes past the fields the box wrote"
        );
        assert_eq!(
            out_of_range.to_string(),
            "value 4294967296 does not fit the 4 bytes of the field it was given to"
        );
    }
}
