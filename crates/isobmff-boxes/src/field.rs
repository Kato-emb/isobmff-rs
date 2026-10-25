//! [`TableEntry`], the entry of a table of fixed-length entries, and the tables of such entries

use alloc::vec::Vec;

use isobmff_core::{Error, FieldReader, FieldWriter, FourCc};

/// Entry of a table whose entries all take the same number of bytes
///
/// A table is a run of entries in a box payload, and a type implementing this
/// is one entry of such a table: a pair of integers, or a four-character code
/// where the codes themselves are the entries. The table functions of this
/// module read and write the table whole.
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
pub(crate) fn entries_len<Entry: TableEntry>(entries: &[Entry]) -> u64 {
    u64::try_from(entries.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(Entry::ENCODED_LEN)
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

    use super::read_to_end;

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
}
