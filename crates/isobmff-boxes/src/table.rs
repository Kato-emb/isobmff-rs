//! [`TableEntry`], the entry of a table of fixed-length entries, and the tables of such entries

use alloc::vec::Vec;
use core::slice::ChunksExact;

use isobmff_core::{Error, FieldReader, FieldWidth, FieldWriter, FourCc};

/// Entry of a table whose entries all take the same number of bytes
///
/// The names are those of ISO/IEC 14496-12, whose Definition of a box calls
/// the run of records its syntax loops over a table and each record an entry
/// ("Each entry in the table gives the number of consecutive samples", §8.6.1.2).
/// A type implementing this is one entry of such a table: a pair of integers,
/// or an integer or a four-character code where the values themselves are the
/// entries. The table functions of this module read and write the table whole.
///
/// The trait takes no context, so it covers only entries every one of which
/// is laid out alike. A table whose entries change shape with the box around
/// them — with its `version`, as in `elst`; with a width a field states, as in
/// `tfra`; with flags, as in `trun` — is read and written by hand in its box.
/// A field that stands alone is not an entry either: the box reads it with
/// [`FieldReader`] and writes it with [`FieldWriter`] directly.
pub(crate) trait TableEntry: Sized {
    /// Number of bytes every entry takes, never zero
    const ENCODED_LEN: u64;

    /// Reads one entry off the front of what `reader` has left
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends inside the entry.
    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error>;

    /// Writes the entry onto the front of what `writer` has left
    ///
    /// # Errors
    ///
    /// * [`TruncatedBuffer`](isobmff_core::ErrorKind::TruncatedBuffer): the
    ///   buffer ends inside the entry.
    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error>;
}

impl TableEntry for FourCc {
    const ENCODED_LEN: u64 = 4;

    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self::new(*reader.read_bytes::<4>()?))
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(self.as_bytes())
    }
}

impl TableEntry for u16 {
    const ENCODED_LEN: u64 = 2;

    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        reader.read_u16()
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u16(*self)
    }
}

impl TableEntry for u32 {
    const ENCODED_LEN: u64 = 4;

    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        reader.read_u32()
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(*self)
    }
}

impl TableEntry for u64 {
    const ENCODED_LEN: u64 = 8;

    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        reader.read_u64()
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u64(*self)
    }
}

/// Reads the 32-bit `entry_count` of a table and then as many entries
///
/// A count of more entries than the rest of the payload holds is refused
/// before any entry is read, as [`read_entries`] refuses it.
///
/// # Errors
///
/// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
///   payload ends inside the count, or holds fewer entries than it counts.
pub(crate) fn read_counted<Entry: TableEntry>(
    reader: &mut FieldReader<'_>,
) -> Result<Vec<Entry>, Error> {
    let entry_count = reader.read_u32()?;

    read_fixed_entries(reader, entry_count, Entry::ENCODED_LEN, Entry::read)
}

/// Reads `entry_count` entries of a table, each with `read`, where every entry takes exactly `entry_len` bytes
///
/// This is how a table whose entries all take the same number of bytes, but
/// whose fields the box around them lays out, is read: `ctts` reads its
/// offsets signed or unsigned as its version says. A count of more entries
/// than the rest of the payload holds is refused before any entry is read, as
/// [`read_entries`] refuses it.
///
/// `entry_len` is never zero, and `read` reads all of an entry's `entry_len`
/// bytes and no more.
///
/// # Errors
///
/// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
///   payload holds fewer entries than `entry_count`.
/// * What `read` reports for an entry.
pub(crate) fn read_fixed_entries<Entry>(
    reader: &mut FieldReader<'_>,
    entry_count: u32,
    entry_len: u64,
    read: impl FnMut(&mut FieldReader<'_>) -> Result<Entry, Error>,
) -> Result<Vec<Entry>, Error> {
    let table_len = u64::from(entry_count).saturating_mul(entry_len);
    // Why not leave the length check to `read_slice`: where `usize` is narrower
    // than `u64`, the length it is given is cut down to `usize::MAX`, and the
    // refusal would name that rather than what the count asks for.
    reader.require(table_len)?;
    let table = reader.read_slice(usize::try_from(table_len).unwrap_or(usize::MAX))?;

    read_whole_entries(
        table.chunks_exact(usize::try_from(entry_len).unwrap_or(usize::MAX)),
        read,
    )
}

/// Reads one entry with `read` off each of `entry_bytes`, the runs of bytes the entries of a table take
///
/// # Errors
///
/// * What `read` reports for an entry.
fn read_whole_entries<Entry>(
    entry_bytes: ChunksExact<'_, u8>,
    mut read: impl FnMut(&mut FieldReader<'_>) -> Result<Entry, Error>,
) -> Result<Vec<Entry>, Error> {
    let mut entries = Vec::with_capacity(entry_bytes.len());
    // Why not read every entry off the reader of the payload: an entry read off
    // a reader of exactly the bytes it takes lets the compiler drop the length
    // check of each of its fields. That holds only where the entry length is a
    // constant; a length the flags of the box set, as in `trun`, keeps the
    // checks and adds a reader per entry.
    for entry in entry_bytes {
        entries.push(read(&mut FieldReader::new(entry))?);
    }

    Ok(entries)
}

/// Reads `entry_count` entries of a table, each with `read`, where every entry takes at least `minimum_len` bytes
///
/// This is how a table whose entries change shape with the box around them
/// is read: the box reads an entry as its version or flags lay it out, and
/// this bounds the table. The count is held against the payload before
/// anything is allocated: a count of more entries than the rest of the
/// payload holds at `minimum_len` each is refused before any entry is read.
/// What the table allocates and how many entries it reads are so bounded by
/// the length of the payload, which the caller sets.
///
/// `minimum_len` is never zero.
///
/// # Errors
///
/// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
///   payload holds fewer entries than `entry_count`.
/// * What `read` reports for an entry.
pub(crate) fn read_entries<Entry>(
    reader: &mut FieldReader<'_>,
    entry_count: u32,
    minimum_len: u64,
    mut read: impl FnMut(&mut FieldReader<'_>) -> Result<Entry, Error>,
) -> Result<Vec<Entry>, Error> {
    reader.require(u64::from(entry_count).saturating_mul(minimum_len))?;
    let mut entries = Vec::with_capacity(usize::try_from(entry_count).unwrap_or_default());
    for _ in 0..entry_count {
        entries.push(read(reader)?);
    }

    Ok(entries)
}

/// Reads the entries of a table that states no count and runs to the end of the payload
///
/// The table allocates room for the entries the rest of the payload holds and
/// no more, and reads no more entries than that, so what it allocates and how
/// many entries it reads are bounded by the length of the payload, which the
/// caller sets.
///
/// # Errors
///
/// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
///   payload ends inside an entry.
pub(crate) fn read_to_end<Entry: TableEntry>(
    reader: &mut FieldReader<'_>,
) -> Result<Vec<Entry>, Error> {
    let rest = reader.remainder();
    let entry_bytes = rest.chunks_exact(usize::try_from(Entry::ENCODED_LEN).unwrap_or(usize::MAX));
    reader.read_slice(rest.len().saturating_sub(entry_bytes.remainder().len()))?;
    let mut entries = read_whole_entries(entry_bytes, Entry::read)?;
    // Why not stop at the whole entries: a payload ending inside an entry is
    // refused with the length its fields need, which only reading the entry off
    // `reader` reports. What is left is shorter than `ENCODED_LEN`, so at most
    // one entry is read here.
    if !reader.remainder().is_empty() {
        entries.push(Entry::read(reader)?);
    }

    Ok(entries)
}

/// Returns how many bytes the entries of a table take, saturating at `u64::MAX`
pub(crate) fn entries_len<Entry: TableEntry>(entries: &[Entry]) -> u64 {
    table_len(entries.len(), Entry::ENCODED_LEN)
}

/// Returns how many bytes `entry_count` entries of `entry_len` bytes each take, saturating at `u64::MAX`
pub(crate) fn table_len(entry_count: usize, entry_len: u64) -> u64 {
    u64::try_from(entry_count)
        .unwrap_or(u64::MAX)
        .saturating_mul(entry_len)
}

/// Returns how many bytes a table and the 32-bit `entry_count` before it take, saturating at `u64::MAX`
pub(crate) fn counted_len<Entry: TableEntry>(entries: &[Entry]) -> u64 {
    entries_len(entries).saturating_add(4)
}

/// Writes the 32-bit `entry_count` of a table, the length of `entries`, and then the entries
///
/// # Errors
///
/// * [`OutOfRange`](isobmff_core::ErrorKind::OutOfRange): the table holds
///   more entries than 32 bits count.
/// * What [`TableEntry::write`] reports for an entry.
pub(crate) fn write_counted<Entry: TableEntry>(
    writer: &mut FieldWriter<'_>,
    entries: &[Entry],
) -> Result<(), Error> {
    write_entry_count(writer, entries.len())?;
    write_entries(writer, entries)
}

/// Writes `entry_count`, the number of entries of a table, as the 32-bit field that counts them
///
/// # Errors
///
/// * [`OutOfRange`](isobmff_core::ErrorKind::OutOfRange): `entry_count` is
///   more than 32 bits count.
/// * [`TruncatedBuffer`](isobmff_core::ErrorKind::TruncatedBuffer): the
///   buffer ends inside the field.
pub(crate) fn write_entry_count(
    writer: &mut FieldWriter<'_>,
    entry_count: usize,
) -> Result<(), Error> {
    writer.write_unsigned(
        FieldWidth::Compact,
        u64::try_from(entry_count).unwrap_or(u64::MAX),
    )
}

/// Writes the entries of a table in the order they are held
///
/// # Errors
///
/// * What [`TableEntry::write`] reports for an entry.
pub(crate) fn write_entries<Entry: TableEntry>(
    writer: &mut FieldWriter<'_>,
    entries: &[Entry],
) -> Result<(), Error> {
    entries.iter().try_for_each(|entry| entry.write(writer))
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{Error, FieldReader, FourCc};

    use super::{read_counted, read_to_end};

    #[test]
    fn a_table_running_to_the_end_reads_every_entry_the_payload_holds() {
        let mut reader = FieldReader::new(b"isomiso2");

        assert_eq!(
            read_to_end::<FourCc>(&mut reader),
            Ok(vec![FourCc::new(*b"isom"), FourCc::new(*b"iso2")])
        );
    }

    #[test]
    fn a_table_running_to_the_end_refuses_a_payload_ending_inside_an_entry() {
        let mut reader = FieldReader::new(b"isomis");

        assert_eq!(
            read_to_end::<FourCc>(&mut reader),
            Err(Error::truncated_payload(8, 6))
        );
    }

    #[test]
    fn a_counted_table_reads_as_many_entries_as_it_counts() {
        let mut reader = FieldReader::new(b"\0\0\0\x02isomiso2avc1");

        assert_eq!(
            (read_counted::<FourCc>(&mut reader), reader.remainder()),
            (
                Ok(vec![FourCc::new(*b"isom"), FourCc::new(*b"iso2")]),
                b"avc1".as_slice()
            )
        );
    }

    #[test]
    fn a_count_of_more_entries_than_the_payload_holds_is_refused_before_any_is_read() {
        let mut reader = FieldReader::new(b"\xff\xff\xff\xffisom");

        assert_eq!(
            read_counted::<FourCc>(&mut reader),
            Err(Error::truncated_payload(4 + 0xffff_ffff * 4, 8))
        );
    }
}
