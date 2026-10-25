//! [`FieldCodec`], a field read and written by its bytes alone, and the tables of such fields

use alloc::vec::Vec;

use isobmff_core::{Error, FieldReader, FieldWriter, FourCc, FullBoxHeader};

/// Value one field of a box payload carries, read and written by its own bytes alone
///
/// A box reads its payload by reading each field off the front in the order
/// its Syntax subclause declares it, and writes it back the same way. A type
/// implementing this is a field whose bytes settle its value with nothing else
/// to go by: an integer, a four-character code, a value type of the
/// specification, the entry of a table.
///
/// The trait takes no context, so a field whose width the `version` of a full
/// box settles is not one: the box reads such a field with
/// [`FieldWidth`](isobmff_core::FieldWidth) and
/// [`read_unsigned`](FieldReader::read_unsigned), and writes it with
/// [`write_unsigned`](FieldWriter::write_unsigned).
pub(crate) trait FieldCodec: Sized {
    /// Reads the field off the front of what `reader` has left
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends inside the field.
    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error>;

    /// Writes the field onto the front of what `writer` has left
    ///
    /// # Errors
    ///
    /// * [`TruncatedBuffer`](isobmff_core::ErrorKind::TruncatedBuffer): the
    ///   buffer ends inside the field.
    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error>;

    /// Returns how many bytes [`write`](Self::write) writes
    fn encoded_len(&self) -> u64;
}

/// [`FieldCodec`] every value of which takes the same number of bytes, as the entry of a table does
pub(crate) trait FixedLengthField: FieldCodec {
    /// Number of bytes every value of the field takes, never zero
    const ENCODED_LEN: u64;
}

impl FieldCodec for u32 {
    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        reader.read_u32()
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(*self)
    }

    fn encoded_len(&self) -> u64 {
        Self::ENCODED_LEN
    }
}

impl FixedLengthField for u32 {
    const ENCODED_LEN: u64 = 4;
}

impl FieldCodec for FourCc {
    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self::new(*reader.read_bytes::<4>()?))
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(self.as_bytes())
    }

    fn encoded_len(&self) -> u64 {
        Self::ENCODED_LEN
    }
}

impl FixedLengthField for FourCc {
    const ENCODED_LEN: u64 = 4;
}

impl FieldCodec for FullBoxHeader {
    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self::from_bytes(reader.read_bytes::<4>()?))
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&self.to_bytes())
    }

    fn encoded_len(&self) -> u64 {
        Self::ENCODED_LEN
    }
}

impl FixedLengthField for FullBoxHeader {
    const ENCODED_LEN: u64 = 4;
}

/// Reads the `count` entries of a table whose count a field before it states
///
/// The payload is held to `count` entries before any is read or room for one
/// is allocated, so what the table allocates, and how many entries it reads,
/// are bounded by the length of the payload, which the caller sets. Bytes past
/// the last entry are left to what follows the table.
///
/// # Errors
///
/// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
///   payload holds fewer than `count` entries.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the first table with a count is that of elst")
)]
pub(crate) fn read_counted<Entry: FixedLengthField>(
    reader: &mut FieldReader<'_>,
    count: u32,
) -> Result<Vec<Entry>, Error> {
    reader.require(u64::from(count).saturating_mul(Entry::ENCODED_LEN))?;

    // Why not `usize::MAX` as a fallback that could be allocated: the payload
    // holds `count` entries of at least one byte, so `count` is no larger than
    // a slice length and always converts.
    let mut entries = Vec::with_capacity(usize::try_from(count).unwrap_or_default());
    for _ in 0..count {
        entries.push(Entry::read(reader)?);
    }

    Ok(entries)
}

/// Reads the entries of a table that states no count and runs to the end of the payload
///
/// The table allocates room for the entries the rest of the payload holds and
/// no more, so what it allocates is bounded by the length of the payload,
/// which the caller sets.
///
/// # Errors
///
/// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
///   payload ends inside an entry.
pub(crate) fn read_to_end<Entry: FixedLengthField>(
    reader: &mut FieldReader<'_>,
) -> Result<Vec<Entry>, Error> {
    let capacity = usize::try_from(Entry::ENCODED_LEN)
        .ok()
        .and_then(|entry_len| reader.remainder().len().checked_div(entry_len))
        .unwrap_or_default();
    let mut entries = Vec::with_capacity(capacity);
    // Why not a guard against an entry that takes nothing: `ENCODED_LEN` is
    // never zero, so every turn takes at least one byte and the loop ends.
    while !reader.remainder().is_empty() {
        entries.push(Entry::read(reader)?);
    }

    Ok(entries)
}

/// Returns how many bytes the entries of a table take, saturating at `u64::MAX`
pub(crate) fn entries_len<Entry: FixedLengthField>(entries: &[Entry]) -> u64 {
    u64::try_from(entries.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(Entry::ENCODED_LEN)
}

/// Writes the entries of a table in the order they are held
///
/// # Errors
///
/// * What [`FieldCodec::write`] reports for an entry.
pub(crate) fn write_entries<Entry: FieldCodec>(
    writer: &mut FieldWriter<'_>,
    entries: &[Entry],
) -> Result<(), Error> {
    entries.iter().try_for_each(|entry| entry.write(writer))
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{Error, FieldReader};

    use super::{read_counted, read_to_end};

    #[test]
    fn a_count_the_payload_cannot_cover_is_refused_before_an_entry_is_read() {
        let mut reader = FieldReader::new(b"\0\0\0\x01");

        assert_eq!(
            read_counted::<u32>(&mut reader, u32::MAX),
            Err(Error::truncated_payload(0x3_ffff_fffc, 4))
        );
    }

    #[test]
    fn a_counted_table_reads_its_count_and_leaves_what_follows() {
        let mut reader = FieldReader::new(b"\0\0\0\x01\0\0\0\x02\xff");

        assert_eq!(read_counted::<u32>(&mut reader, 2), Ok(vec![1, 2]));
        assert_eq!(reader.remainder(), b"\xff");
    }

    #[test]
    fn a_table_running_to_the_end_reads_every_entry_the_payload_holds() {
        let mut reader = FieldReader::new(b"\0\0\0\x01\0\0\0\x02");

        assert_eq!(read_to_end::<u32>(&mut reader), Ok(vec![1, 2]));
    }

    #[test]
    fn a_table_running_to_the_end_refuses_a_payload_ending_inside_an_entry() {
        let mut reader = FieldReader::new(b"\0\0\0\x01\0\0");

        assert_eq!(
            read_to_end::<u32>(&mut reader),
            Err(Error::truncated_payload(8, 6))
        );
    }
}
