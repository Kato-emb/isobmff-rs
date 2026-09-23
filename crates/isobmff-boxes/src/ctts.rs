//! [`CompositionOffsetBox`] (`ctts`), ISO/IEC 14496-12 §8.6.1.3

use alloc::vec::Vec;
use core::slice;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWidth, FieldWriter,
    FullBoxFields, FullBoxFlags,
};

use crate::trun::CompositionTimeOffset;

/// Length of the fields that precede the entries
const FIXED_FIELDS_LEN: u64 = 8;

/// Length of one entry of the table
const ENTRY_LEN: u64 = 8;

/// One entry of the table a [`CompositionOffsetBox`] holds
///
/// The entry counts the samples that follow one another with the same offset
/// from their decode time to their composition time and states that offset, so
/// the table run-length codes an offset per sample.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CompositionOffsetEntry {
    sample_count: u32,
    sample_offset: CompositionTimeOffset,
}

impl CompositionOffsetEntry {
    /// Creates the entry from the samples it counts and the offset they share
    #[must_use]
    pub const fn new(sample_count: u32, sample_offset: CompositionTimeOffset) -> Self {
        Self {
            sample_count,
            sample_offset,
        }
    }

    /// Returns how many samples in a row carry this offset
    #[must_use]
    pub const fn sample_count(&self) -> u32 {
        self.sample_count
    }

    /// Returns the offset those samples carry, in the media time scale
    #[must_use]
    pub const fn sample_offset(&self) -> CompositionTimeOffset {
        self.sample_offset
    }
}

/// Box that states the offset from the decode time of every sample of a track to its composition time
///
/// [`CompositionOffsetBox`] (`ctts`), ISO/IEC 14496-12 §8.6.1.3. The
/// composition time of a sample is its decode time plus the offset the table
/// states for it, and a track whose samples are all composed when they are
/// decoded carries no such box.
///
/// The `entry_count` field is not held: it counts the entries, so it is derived
/// on the way out. On the way in a count that disagrees with the entries fails
/// the box.
///
/// The version is not held: it selects whether the offsets are written signed
/// or unsigned, so [`encode_payload`](BoxEncode::encode_payload) writes version
/// 0 unless an entry carries a negative offset.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{CompositionOffsetBox, CompositionOffsetEntry, CompositionTimeOffset};
///
/// let offset = |value| CompositionTimeOffset::new(value).unwrap();
///
/// // Two samples composed 512 units late, then one composed when decoded
/// assert_eq!(
///     CompositionOffsetBox::from_offsets([offset(512), offset(512), offset(0)]),
///     CompositionOffsetBox::new(vec![
///         CompositionOffsetEntry::new(2, offset(512)),
///         CompositionOffsetEntry::new(1, offset(0)),
///     ])
/// );
///
/// // An offset below zero beside one past `i32::MAX` builds nothing
/// assert_eq!(
///     CompositionOffsetBox::from_offsets([offset(-8), offset(1 << 31)]),
///     None
/// );
/// ```
#[doc(alias = "ctts")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct CompositionOffsetBox {
    entries: Vec<CompositionOffsetEntry>,
}

impl CompositionOffsetBox {
    /// Creates the box from the entries it states the offsets with
    ///
    /// Returns `None` when one entry carries a negative offset while another
    /// carries one past [`i32::MAX`], which leaves no version able to write
    /// both.
    #[must_use]
    pub fn new(entries: Vec<CompositionOffsetEntry>) -> Option<Self> {
        CompositionTimeOffset::version_writing(entries.iter().map(|entry| entry.sample_offset))?;

        Some(Self { entries })
    }

    /// Creates the box from the offset of every sample in turn, run-length coded
    ///
    /// Samples following one another with the same offset are counted by one
    /// entry, as far as one entry counts; a run past that is counted by the
    /// next. Returns `None` as [`new`](Self::new) does.
    #[must_use]
    pub fn from_offsets(offsets: impl IntoIterator<Item = CompositionTimeOffset>) -> Option<Self> {
        let mut entries: Vec<CompositionOffsetEntry> = Vec::new();
        for sample_offset in offsets {
            let counted = entries
                .last_mut()
                .filter(|entry| entry.sample_offset == sample_offset)
                .and_then(|entry| {
                    entry.sample_count = entry.sample_count.checked_add(1)?;

                    Some(())
                });
            if counted.is_none() {
                entries.push(CompositionOffsetEntry::new(1, sample_offset));
            }
        }

        Self::new(entries)
    }

    /// Returns the entries, in the order the decode timeline runs
    #[must_use]
    pub fn entries(&self) -> &[CompositionOffsetEntry] {
        &self.entries
    }

    /// Returns the composition time offset of every sample in turn
    ///
    /// The offset an entry states comes out once per sample the entry counts.
    pub fn offsets(&self) -> impl Iterator<Item = CompositionTimeOffset> + '_ {
        Offsets {
            entries: self.entries.iter(),
            sample_offset: None,
            remaining: 0,
        }
    }
}

/// The composition time offsets of the samples of a track read one after another
struct Offsets<'ctts> {
    /// Entries still to be read
    entries: slice::Iter<'ctts, CompositionOffsetEntry>,
    /// Offset of the entry being read, `None` before the first
    sample_offset: Option<CompositionTimeOffset>,
    /// Samples of the entry being read that have still to come out
    remaining: u32,
}

impl Iterator for Offsets<'_> {
    type Item = CompositionTimeOffset;

    fn next(&mut self) -> Option<CompositionTimeOffset> {
        loop {
            if let Some(remaining) = self.remaining.checked_sub(1) {
                self.remaining = remaining;

                return self.sample_offset;
            }
            let entry = self.entries.next()?;
            self.sample_offset = Some(entry.sample_offset);
            self.remaining = entry.sample_count;
        }
    }
}

impl BoxDefinition for CompositionOffsetBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"ctts");
}

impl BoxDecode for CompositionOffsetBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0 or 1.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
    ///   ends inside a field of the box or inside one of its entries.
    /// * [`EntryCountMismatch`](isobmff_core::ErrorKind::EntryCountMismatch): the
    ///   `entry_count` field disagrees with the entries that follow it.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version > 1 {
            return Err(Error::unsupported_version(version));
        }

        let declared = u64::from(reader.read_u32()?);

        let mut entries = Vec::new();
        while !reader.remainder().is_empty() {
            entries.push(CompositionOffsetEntry {
                sample_count: reader.read_u32()?,
                sample_offset: CompositionTimeOffset::read(reader, version)?,
            });
        }

        let actual = entries.len() as u64;
        if actual != declared {
            return Err(Error::entry_count_mismatch(declared, actual));
        }

        Ok(Self { entries })
    }
}

impl BoxEncode for CompositionOffsetBox {
    fn payload_len(&self) -> u64 {
        let entries = (self.entries.len() as u64).saturating_mul(ENTRY_LEN);

        FIXED_FIELDS_LEN.saturating_add(entries)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        // Why not unwrap: `CompositionOffsetBox::new` refuses entries no one
        // version writes, and were one to slip through, version 1 refuses the
        // offset past its range when it is written.
        let version = CompositionTimeOffset::version_writing(
            self.entries.iter().map(|entry| entry.sample_offset),
        )
        .unwrap_or(1);

        writer.write_bytes(&FullBoxFields::new(version, FullBoxFlags::ZERO).to_bytes())?;
        let entry_count = self.entries.len() as u64;
        // Why not saturate silently: an entry count past `u32` cannot be written
        // at all, and the box has already declared a length built from it, so
        // this stands for a `Vec` no target can hold.
        writer.write_unsigned(FieldWidth::Compact, entry_count)?;

        for entry in &self.entries {
            writer.write_u32(entry.sample_count)?;
            entry.sample_offset.write(writer, version)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::{CompositionOffsetBox, CompositionOffsetEntry};
    use crate::trun::CompositionTimeOffset;

    /// Offset of `value` units, which lies within what either version writes
    fn offset(value: i64) -> CompositionTimeOffset {
        CompositionTimeOffset::new(value).unwrap()
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(composition_offset: &CompositionOffsetBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(composition_offset.payload_len()).unwrap()];
        composition_offset.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_of_offsets_at_or_past_zero_is_written_unsigned_and_reads_back() {
        let composition_offset = CompositionOffsetBox::new(vec![
            CompositionOffsetEntry::new(2, offset(0xffff_ffff)),
            CompositionOffsetEntry::new(1, offset(0)),
        ])
        .unwrap();

        let payload = encoded_payload(&composition_offset);

        assert_eq!(
            payload,
            b"\0\0\0\0\0\0\0\x02\0\0\0\x02\xff\xff\xff\xff\0\0\0\x01\0\0\0\0"
        );
        assert_eq!(
            CompositionOffsetBox::decode_payload(&payload).unwrap(),
            composition_offset
        );
    }

    #[test]
    fn a_box_holding_a_negative_offset_is_written_signed_and_reads_back() {
        let composition_offset = CompositionOffsetBox::new(vec![
            CompositionOffsetEntry::new(1, offset(-2)),
            CompositionOffsetEntry::new(3, offset(8)),
        ])
        .unwrap();

        let payload = encoded_payload(&composition_offset);

        assert_eq!(
            payload,
            b"\x01\0\0\0\0\0\0\x02\0\0\0\x01\xff\xff\xff\xfe\0\0\0\x03\0\0\0\x08"
        );
        assert_eq!(
            CompositionOffsetBox::decode_payload(&payload).unwrap(),
            composition_offset
        );
    }

    #[test]
    fn offsets_no_one_version_writes_build_no_box() {
        assert_eq!(
            CompositionOffsetBox::new(vec![
                CompositionOffsetEntry::new(1, offset(-1)),
                CompositionOffsetEntry::new(1, offset(1 << 31)),
            ]),
            None
        );
    }

    #[test]
    fn samples_following_one_another_with_one_offset_are_counted_by_one_entry() {
        assert_eq!(
            CompositionOffsetBox::from_offsets([offset(8), offset(8), offset(0), offset(8)]),
            CompositionOffsetBox::new(vec![
                CompositionOffsetEntry::new(2, offset(8)),
                CompositionOffsetEntry::new(1, offset(0)),
                CompositionOffsetEntry::new(1, offset(8)),
            ])
        );
    }

    #[test]
    fn the_offset_of_an_entry_is_read_once_per_sample_it_counts() {
        let composition_offset = CompositionOffsetBox::new(vec![
            CompositionOffsetEntry::new(2, offset(8)),
            CompositionOffsetEntry::new(0, offset(4)),
            CompositionOffsetEntry::new(1, offset(-2)),
        ])
        .unwrap();

        assert_eq!(
            composition_offset.offsets().collect::<Vec<_>>(),
            [offset(8), offset(8), offset(-2)]
        );
    }

    #[test]
    fn a_count_that_disagrees_with_the_entries_is_rejected() {
        let mut payload = encoded_payload(
            &CompositionOffsetBox::new(vec![CompositionOffsetEntry::new(1, offset(8))]).unwrap(),
        );
        payload
            .get_mut(4..8)
            .unwrap()
            .copy_from_slice(&4_u32.to_be_bytes());

        assert_eq!(
            CompositionOffsetBox::decode_payload(&payload),
            Err(Error::entry_count_mismatch(4, 1))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let payload = b"\x02\0\0\0\0\0\0\0";

        assert_eq!(
            CompositionOffsetBox::decode_payload(payload),
            Err(Error::unsupported_version(2))
        );
    }
}
