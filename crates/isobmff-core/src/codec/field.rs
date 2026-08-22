//! [`FieldReader`] and [`FieldWriter`], the fields a box payload of ISO/IEC 14496-12 §4.2 is made of

use core::mem;

use crate::error::{Error, byte_count};

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
/// A box whose payload is fixed has every byte of it claimed by a field, which
/// is what [`decode_payload`](crate::BoxDecode::decode_payload) holds it to
/// once the fields are read. One whose last field runs to the end of the
/// payload claims what is left with
/// [`take_remainder`](Self::take_remainder).
///
/// # Examples
///
/// ```
/// use isobmff_core::{Error, FieldReader};
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
///     Err(Error::truncated_payload(4, 3))
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
    /// * [`TruncatedPayload`](crate::ErrorKind::TruncatedPayload): the payload ends
    ///   inside the field.
    pub fn read_bytes<const N: usize>(&mut self) -> Result<&'payload [u8; N], Error> {
        let needed = self.consumed.saturating_add(byte_count(N));
        let rest = self.rest;
        let (field, tail) = rest.split_first_chunk::<N>().ok_or_else(|| {
            Error::truncated_payload(needed, self.consumed.saturating_add(byte_count(rest.len())))
        })?;

        self.rest = tail;
        self.consumed = needed;

        Ok(field)
    }

    /// Reads the next field as a 16-bit unsigned integer
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](crate::ErrorKind::TruncatedPayload): the payload ends
    ///   inside the field.
    pub fn read_u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_be_bytes(*self.read_bytes::<2>()?))
    }

    /// Reads the next field as a 16-bit signed integer
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](crate::ErrorKind::TruncatedPayload): the payload ends
    ///   inside the field.
    pub fn read_i16(&mut self) -> Result<i16, Error> {
        Ok(i16::from_be_bytes(*self.read_bytes::<2>()?))
    }

    /// Reads the next field as a 32-bit unsigned integer
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](crate::ErrorKind::TruncatedPayload): the payload ends
    ///   inside the field.
    pub fn read_u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_be_bytes(*self.read_bytes::<4>()?))
    }

    /// Reads the next field as a 32-bit signed integer
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](crate::ErrorKind::TruncatedPayload): the payload ends
    ///   inside the field.
    pub fn read_i32(&mut self) -> Result<i32, Error> {
        Ok(i32::from_be_bytes(*self.read_bytes::<4>()?))
    }

    /// Reads the next field as a 64-bit unsigned integer
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](crate::ErrorKind::TruncatedPayload): the payload ends
    ///   inside the field.
    pub fn read_u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_be_bytes(*self.read_bytes::<8>()?))
    }

    /// Reads the next field as an unsigned integer of the width the box settled
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](crate::ErrorKind::TruncatedPayload): the payload ends
    ///   inside the field.
    pub fn read_unsigned(&mut self, width: FieldWidth) -> Result<u64, Error> {
        match width {
            FieldWidth::Compact => Ok(u64::from(self.read_u32()?)),
            FieldWidth::Extended => self.read_u64(),
        }
    }

    /// Requires the payload to hold `bytes` more than the fields have taken
    ///
    /// A box asks this where the payload states how much is coming — a count of
    /// rows and the length of one. Nothing is taken and nothing is read: the
    /// cursor stands where it stood.
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](crate::ErrorKind::TruncatedPayload): the payload holds
    ///   fewer than `bytes` past the fields already read.
    pub fn require(&self, bytes: u64) -> Result<(), Error> {
        let remaining = byte_count(self.rest.len());
        if bytes <= remaining {
            return Ok(());
        }

        Err(Error::truncated_payload(
            self.consumed.saturating_add(bytes),
            self.consumed.saturating_add(remaining),
        ))
    }

    /// Returns the bytes of the payload no field has taken
    ///
    /// The cursor stands where it stood, so this is what a box reads a run of
    /// fields against — whether one more of them is there at all. A field that
    /// runs to the end of the payload claims those bytes with
    /// [`take_remainder`](Self::take_remainder) instead.
    #[must_use]
    pub fn remainder(&self) -> &'payload [u8] {
        self.rest
    }

    /// Takes the rest of the payload as the field that runs to its end
    ///
    /// This is how a variable-length field is bounded: the field is whatever no
    /// field before it took. The cursor reaches the end of the payload, so
    /// nothing is left for [`finish`](Self::finish) to refuse.
    #[must_use]
    pub fn take_remainder(&mut self) -> &'payload [u8] {
        let rest = mem::take(&mut self.rest);
        self.consumed = self.consumed.saturating_add(byte_count(rest.len()));

        rest
    }

    /// Reports the payload as read whole, which every field of a fixed box has claimed
    ///
    /// # Errors
    ///
    /// * [`TrailingPayload`](crate::ErrorKind::TrailingPayload): the payload holds
    ///   bytes past the fields of the box.
    pub fn finish(self) -> Result<(), Error> {
        if self.rest.is_empty() {
            return Ok(());
        }

        Err(Error::trailing_payload(
            self.consumed,
            self.consumed.saturating_add(byte_count(self.rest.len())),
        ))
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
/// promised. [`encode_payload`](crate::BoxEncode::encode_payload) is where the
/// two are held against each other: a buffer with bytes to spare means the
/// declared length is longer than the fields, as running out of buffer
/// mid-field means it is shorter.
///
/// # Examples
///
/// ```
/// use isobmff_core::{Error, FieldWriter};
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
///     Err(Error::truncated_buffer(4, 3))
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
    /// * [`TruncatedBuffer`](crate::ErrorKind::TruncatedBuffer): the buffer ends
    ///   inside the field.
    pub fn write_bytes<const N: usize>(&mut self, bytes: &[u8; N]) -> Result<(), Error> {
        let needed = self.written.saturating_add(byte_count(N));
        let available = self.written.saturating_add(byte_count(self.rest.len()));
        // Why not leaving the buffer in place: the field does not fit, so the
        // box abandons the writer, and what is held back would be a buffer no
        // field can be written into anyway.
        let Some((field, tail)) = mem::take(&mut self.rest).split_first_chunk_mut::<N>() else {
            return Err(Error::truncated_buffer(needed, available));
        };

        *field = *bytes;
        self.rest = tail;
        self.written = needed;

        Ok(())
    }

    /// Writes the next field, which occupies the length of `bytes`
    ///
    /// # Errors
    ///
    /// * [`TruncatedBuffer`](crate::ErrorKind::TruncatedBuffer): the buffer ends
    ///   inside the field.
    pub fn write_slice(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let needed = self.written.saturating_add(byte_count(bytes.len()));
        let available = self.written.saturating_add(byte_count(self.rest.len()));
        // Why not leaving the buffer in place: the field does not fit, so the
        // box abandons the writer, and what is held back would be a buffer no
        // field can be written into anyway.
        let Some((field, tail)) = mem::take(&mut self.rest).split_at_mut_checked(bytes.len())
        else {
            return Err(Error::truncated_buffer(needed, available));
        };

        field.copy_from_slice(bytes);
        self.rest = tail;
        self.written = needed;

        Ok(())
    }

    /// Writes the next field as a 16-bit unsigned integer
    ///
    /// # Errors
    ///
    /// * [`TruncatedBuffer`](crate::ErrorKind::TruncatedBuffer): the buffer ends
    ///   inside the field.
    pub fn write_u16(&mut self, value: u16) -> Result<(), Error> {
        self.write_bytes(&value.to_be_bytes())
    }

    /// Writes the next field as a 16-bit signed integer
    ///
    /// # Errors
    ///
    /// * [`TruncatedBuffer`](crate::ErrorKind::TruncatedBuffer): the buffer ends
    ///   inside the field.
    pub fn write_i16(&mut self, value: i16) -> Result<(), Error> {
        self.write_bytes(&value.to_be_bytes())
    }

    /// Writes the next field as a 32-bit unsigned integer
    ///
    /// # Errors
    ///
    /// * [`TruncatedBuffer`](crate::ErrorKind::TruncatedBuffer): the buffer ends
    ///   inside the field.
    pub fn write_u32(&mut self, value: u32) -> Result<(), Error> {
        self.write_bytes(&value.to_be_bytes())
    }

    /// Writes the next field as a 32-bit signed integer
    ///
    /// # Errors
    ///
    /// * [`TruncatedBuffer`](crate::ErrorKind::TruncatedBuffer): the buffer ends
    ///   inside the field.
    pub fn write_i32(&mut self, value: i32) -> Result<(), Error> {
        self.write_bytes(&value.to_be_bytes())
    }

    /// Writes the next field as a 64-bit unsigned integer
    ///
    /// # Errors
    ///
    /// * [`TruncatedBuffer`](crate::ErrorKind::TruncatedBuffer): the buffer ends
    ///   inside the field.
    pub fn write_u64(&mut self, value: u64) -> Result<(), Error> {
        self.write_bytes(&value.to_be_bytes())
    }

    /// Writes the next field as an unsigned integer of the width the box settled
    ///
    /// # Errors
    ///
    /// * [`TruncatedBuffer`](crate::ErrorKind::TruncatedBuffer): the buffer ends
    ///   inside the field.
    /// * [`OutOfRange`](crate::ErrorKind::OutOfRange): `value` is wider than the
    ///   field, which leaves nothing to write.
    pub fn write_unsigned(&mut self, width: FieldWidth, value: u64) -> Result<(), Error> {
        match width {
            FieldWidth::Compact => {
                let narrow = u32::try_from(value).map_err(|_| Error::out_of_range(value, width))?;

                self.write_u32(narrow)
            }
            FieldWidth::Extended => self.write_u64(value),
        }
    }

    /// Takes the rest of the buffer for the field that runs to its end
    ///
    /// The mirror of [`FieldReader::take_remainder`]: the bytes are the field,
    /// and the cursor reaches the end of the buffer, so nothing is left for
    /// [`finish`](Self::finish) to refuse. What the field does not write into
    /// keeps whatever the buffer held.
    #[must_use]
    pub fn take_remainder(&mut self) -> &'buffer mut [u8] {
        let rest = mem::take(&mut self.rest);
        self.written = self.written.saturating_add(byte_count(rest.len()));

        rest
    }

    /// Reports the buffer as written whole, which every field of a fixed box has claimed
    ///
    /// # Errors
    ///
    /// * [`TrailingBuffer`](crate::ErrorKind::TrailingBuffer): the buffer holds
    ///   bytes past the fields the box claimed.
    pub fn finish(self) -> Result<(), Error> {
        if self.rest.is_empty() {
            return Ok(());
        }

        Err(Error::trailing_buffer(
            self.written,
            self.written.saturating_add(byte_count(self.rest.len())),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{Error, FieldReader, FieldWidth, FieldWriter};

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
        assert_eq!(reader.read_u32(), Err(Error::truncated_payload(8, 6)));
    }

    #[test]
    fn a_payload_with_bytes_past_its_fields_is_refused() {
        let mut reader = FieldReader::new(b"\x01\x02\x03\x04\x05");

        assert_eq!(reader.read_u32(), Ok(0x0102_0304));
        assert_eq!(reader.finish(), Err(Error::trailing_payload(4, 5)));
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
    fn a_field_running_to_the_end_of_the_payload_claims_what_is_left_of_it() {
        let mut reader = FieldReader::new(b"\0\x02rest of the payload");

        assert_eq!(reader.read_u16(), Ok(2));
        assert_eq!(reader.take_remainder(), b"rest of the payload");
        assert_eq!(reader.finish(), Ok(()));
    }

    #[test]
    fn a_payload_that_cannot_cover_what_it_declares_is_refused_before_it_is_read() {
        let mut reader = FieldReader::new(b"\0\x03\x01\x02");

        assert_eq!(reader.read_u16(), Ok(3));
        assert_eq!(reader.require(12), Err(Error::truncated_payload(14, 4)));
    }

    #[test]
    fn a_payload_holding_what_it_declares_is_let_on_to_the_fields() {
        let mut reader = FieldReader::new(b"\0\x03\x01\x02");

        assert_eq!(reader.read_u16(), Ok(3));
        assert_eq!(reader.require(2), Ok(()));
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
            Err(Error::truncated_buffer(8, 6))
        );
    }

    #[test]
    fn a_slice_field_writes_the_bytes_it_carries() {
        let mut buffer = [0; 6];
        let mut writer = FieldWriter::new(&mut buffer);

        assert_eq!(writer.write_u16(2), Ok(()));
        assert_eq!(writer.write_slice(b"rest"), Ok(()));
        assert_eq!(writer.finish(), Ok(()));
        assert_eq!(buffer, *b"\0\x02rest");
    }

    #[test]
    fn a_slice_field_running_past_the_buffer_names_what_the_fields_required() {
        let mut buffer = [0; 4];
        let mut writer = FieldWriter::new(&mut buffer);

        assert_eq!(writer.write_u16(2), Ok(()));
        assert_eq!(
            writer.write_slice(b"abc"),
            Err(Error::truncated_buffer(5, 4))
        );
    }

    #[test]
    fn a_buffer_with_bytes_past_the_fields_written_is_refused() {
        let mut buffer = [0; 5];
        let mut writer = FieldWriter::new(&mut buffer);

        assert_eq!(writer.write_u32(0x0102_0304), Ok(()));
        assert_eq!(writer.finish(), Err(Error::trailing_buffer(4, 5)));
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
            Err(Error::out_of_range(0x1_0000_0000, FieldWidth::Compact))
        );
        assert_eq!(buffer, [0xff; 4]);
    }

    #[test]
    fn a_field_running_to_the_end_of_the_buffer_claims_what_is_left_of_it() {
        let mut buffer = [0; 6];
        let mut writer = FieldWriter::new(&mut buffer);

        assert_eq!(writer.write_u16(2), Ok(()));
        writer.take_remainder().copy_from_slice(b"tail");

        assert_eq!(writer.finish(), Ok(()));
        assert_eq!(&buffer, b"\0\x02tail");
    }
}
