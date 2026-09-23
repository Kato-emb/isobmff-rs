//! [`TrackFragmentRandomAccessBox`] (`tfra`), ISO/IEC 14496-12 §8.8.10

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWidth, FieldWriter,
    FullBoxFields, FullBoxFlags,
};

/// Length of the fields that precede the entries
const FIXED_FIELDS_LEN: u64 = 16;

/// Length of the time and the `moof` offset of one entry when version 0 carries them in 32 bits
const TIME_AND_OFFSET_LEN_VERSION_0: u64 = 8;

/// Length of the time and the `moof` offset of one entry when version 1 carries them in 64 bits
const TIME_AND_OFFSET_LEN_VERSION_1: u64 = 16;

/// Mask of the 2 bits one `length_size_of_*` field occupies
const LENGTH_SIZE_MASK: u8 = 0b11;

/// One entry of the table a [`TrackFragmentRandomAccessBox`] holds
///
/// The entry names a sync sample of the track: the presentation time it
/// starts at, the `moof` holding it, and its place inside that `moof` — the
/// `traf`, the `trun` in that `traf`, and the sample in that `trun`, each
/// counted from 1.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TrackFragmentRandomAccessEntry {
    time: u64,
    moof_offset: u64,
    traf_number: u32,
    trun_number: u32,
    sample_number: u32,
}

impl TrackFragmentRandomAccessEntry {
    /// Creates the entry from the time of the sync sample and where it lies
    #[must_use]
    pub const fn new(
        time: u64,
        moof_offset: u64,
        traf_number: u32,
        trun_number: u32,
        sample_number: u32,
    ) -> Self {
        Self {
            time,
            moof_offset,
            traf_number,
            trun_number,
            sample_number,
        }
    }

    /// Returns the presentation time of the sync sample, in the media time scale of the track
    #[must_use]
    pub const fn time(&self) -> u64 {
        self.time
    }

    /// Returns the offset from the start of the file of the `moof` holding the sync sample
    #[must_use]
    pub const fn moof_offset(&self) -> u64 {
        self.moof_offset
    }

    /// Returns the number of the `traf` holding the sync sample, counted from 1 in its `moof`
    #[must_use]
    pub const fn traf_number(&self) -> u32 {
        self.traf_number
    }

    /// Returns the number of the `trun` holding the sync sample, counted from 1 in its `traf`
    #[must_use]
    pub const fn trun_number(&self) -> u32 {
        self.trun_number
    }

    /// Returns the number of the sync sample, counted from 1 in its `trun`
    #[must_use]
    pub const fn sample_number(&self) -> u32 {
        self.sample_number
    }
}

/// Box that lists the sync samples of one track with where in the file they lie
///
/// [`TrackFragmentRandomAccessBox`] (`tfra`), ISO/IEC 14496-12 §8.8.10. A
/// reader looks a time up in it and restarts at the `moof` the entry names,
/// without reading the fragments before it. A box with no entries states that
/// every sample of the track is a sync sample.
///
/// The version is not held: it selects how wide the time and the `moof` offset
/// of every entry are written, so [`encode_payload`](BoxEncode::encode_payload)
/// picks the narrower one whenever every entry fits in 32 bits. The
/// `length_size_of_traf_num`, `length_size_of_trun_num` and
/// `length_size_of_sample_num` fields are not held either: each is written as
/// the fewest bytes the largest number of its column needs. The
/// `number_of_entry` field counts the entries, so it is derived on the way
/// out, and on the way in a count that disagrees with the entries fails the
/// box. The reserved bits are read as nothing and written as zero.
#[doc(alias = "tfra")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TrackFragmentRandomAccessBox {
    track_id: u32,
    entries: Vec<TrackFragmentRandomAccessEntry>,
}

impl TrackFragmentRandomAccessBox {
    /// Creates the box from the track it lists and the entries of its sync samples
    #[must_use]
    pub const fn new(track_id: u32, entries: Vec<TrackFragmentRandomAccessEntry>) -> Self {
        Self { track_id, entries }
    }

    /// Returns the ID of the track whose sync samples the box lists
    #[must_use]
    pub const fn track_id(&self) -> u32 {
        self.track_id
    }

    /// Returns the entries, in the order they came
    #[must_use]
    pub fn entries(&self) -> &[TrackFragmentRandomAccessEntry] {
        &self.entries
    }

    /// Returns the version whose field width carries the times and offsets of this box
    fn version(&self) -> u8 {
        let within_32_bits = self.entries.iter().all(|entry| {
            entry.time <= u64::from(u32::MAX) && entry.moof_offset <= u64::from(u32::MAX)
        });

        if within_32_bits { 0 } else { 1 }
    }

    /// Returns the width the given version carries the times and offsets at
    const fn field_width(version: u8) -> FieldWidth {
        match version {
            0 => FieldWidth::Compact,
            _ => FieldWidth::Extended,
        }
    }

    /// Returns the `length_size_of_*` fields of the traf, trun and sample numbers, in that order
    fn length_sizes(&self) -> [u8; 3] {
        let (traf_numbers, trun_numbers, sample_numbers) = self.entries.iter().fold(
            (0, 0, 0),
            |(traf_number, trun_number, sample_number), entry| {
                (
                    traf_number.max(entry.traf_number),
                    trun_number.max(entry.trun_number),
                    sample_number.max(entry.sample_number),
                )
            },
        );

        [traf_numbers, trun_numbers, sample_numbers].map(|largest| match largest {
            0..=0xff => 0,
            0x100..=0xffff => 1,
            0x1_0000..=0xff_ffff => 2,
            _ => 3,
        })
    }
}

/// Returns how many bytes a number whose `length_size_of_*` field is `length_size` occupies
fn number_len(length_size: u8) -> usize {
    usize::from(length_size & LENGTH_SIZE_MASK).saturating_add(1)
}

/// Reads a number written in the bytes its `length_size_of_*` field states
fn read_number(reader: &mut FieldReader<'_>, length_size: u8) -> Result<u32, Error> {
    let bytes = reader.read_slice(number_len(length_size))?;

    Ok(bytes
        .iter()
        .fold(0, |number, &byte| number << 8 | u32::from(byte)))
}

/// Writes a number in the bytes its `length_size_of_*` field states
fn write_number(writer: &mut FieldWriter<'_>, number: u32, length_size: u8) -> Result<(), Error> {
    let bytes = number.to_be_bytes();
    let skipped = bytes.len().saturating_sub(number_len(length_size));

    writer.write_slice(bytes.get(skipped..).unwrap_or(&bytes))
}

impl BoxDefinition for TrackFragmentRandomAccessBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tfra");
}

impl BoxDecode for TrackFragmentRandomAccessBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0 or 1.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
    ///   ends inside a field of the box or inside one of its entries.
    /// * [`EntryCountMismatch`](isobmff_core::ErrorKind::EntryCountMismatch): the
    ///   `number_of_entry` field disagrees with the entries that follow it.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version > 1 {
            return Err(Error::unsupported_version(version));
        }

        let track_id = reader.read_u32()?;
        let &[_, _, _, length_sizes] = reader.read_bytes::<4>()?;
        let declared = u64::from(reader.read_u32()?);
        let width = Self::field_width(version);

        let mut entries = Vec::new();
        while !reader.remainder().is_empty() {
            entries.push(TrackFragmentRandomAccessEntry {
                time: reader.read_unsigned(width)?,
                moof_offset: reader.read_unsigned(width)?,
                traf_number: read_number(reader, length_sizes >> 4)?,
                trun_number: read_number(reader, length_sizes >> 2)?,
                sample_number: read_number(reader, length_sizes)?,
            });
        }

        let actual = entries.len() as u64;
        if actual != declared {
            return Err(Error::entry_count_mismatch(declared, actual));
        }

        Ok(Self { track_id, entries })
    }
}

impl BoxEncode for TrackFragmentRandomAccessBox {
    fn payload_len(&self) -> u64 {
        let time_and_offset = if self.version() == 0 {
            TIME_AND_OFFSET_LEN_VERSION_0
        } else {
            TIME_AND_OFFSET_LEN_VERSION_1
        };
        let numbers = self.length_sizes().map(number_len).iter().sum::<usize>() as u64;
        let entry_len = numbers.saturating_add(time_and_offset);
        let entries = (self.entries.len() as u64).saturating_mul(entry_len);

        FIXED_FIELDS_LEN.saturating_add(entries)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let version = self.version();
        let width = Self::field_width(version);
        let [traf_length_size, trun_length_size, sample_length_size] = self.length_sizes();

        writer.write_bytes(&FullBoxFields::new(version, FullBoxFlags::ZERO).to_bytes())?;
        writer.write_u32(self.track_id)?;
        writer.write_bytes(&[
            0,
            0,
            0,
            traf_length_size << 4 | trun_length_size << 2 | sample_length_size,
        ])?;
        let number_of_entry = self.entries.len() as u64;
        // Why not saturate silently: an entry count past `u32` cannot be written
        // at all, and the box has already declared a length built from it, so
        // this stands for a `Vec` no target can hold.
        writer.write_unsigned(FieldWidth::Compact, number_of_entry)?;

        for entry in &self.entries {
            writer.write_unsigned(width, entry.time)?;
            writer.write_unsigned(width, entry.moof_offset)?;
            write_number(writer, entry.traf_number, traf_length_size)?;
            write_number(writer, entry.trun_number, trun_length_size)?;
            write_number(writer, entry.sample_number, sample_length_size)?;
        }

        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::{TrackFragmentRandomAccessBox, TrackFragmentRandomAccessEntry};

    /// Random access box of track 1 listing two sync samples, one per fragment
    pub(crate) fn track_fragment_random_access() -> TrackFragmentRandomAccessBox {
        TrackFragmentRandomAccessBox::new(
            1,
            vec![
                TrackFragmentRandomAccessEntry::new(0, 1_000, 1, 1, 1),
                TrackFragmentRandomAccessEntry::new(3_000, 5_000, 1, 1, 1),
            ],
        )
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(random_access: &TrackFragmentRandomAccessBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(random_access.payload_len()).unwrap()];
        random_access.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn an_entry_is_written_in_the_fewest_bytes_its_columns_need() {
        let random_access = TrackFragmentRandomAccessBox::new(
            7,
            vec![TrackFragmentRandomAccessEntry::new(
                0x0102, 0x0304, 0x0506, 0x07, 0x08_090a,
            )],
        );

        let payload = encoded_payload(&random_access);

        assert_eq!(
            payload,
            b"\0\0\0\0\0\0\0\x07\0\0\0\x12\0\0\0\x01\
              \0\0\x01\x02\0\0\x03\x04\x05\x06\x07\x08\x09\x0a"
        );
        assert_eq!(
            TrackFragmentRandomAccessBox::decode_payload(&payload).unwrap(),
            random_access
        );
    }

    #[test]
    fn every_number_width_reads_back_at_either_version() {
        let widths = [(0xff, 0), (0xffff, 1), (0xff_ffff, 2), (u32::MAX, 3)];
        let times = [(u64::from(u32::MAX), 0), (u64::from(u32::MAX) + 1, 1)];

        for (largest, length_size) in widths {
            for (time, version) in times {
                let random_access = TrackFragmentRandomAccessBox::new(
                    1,
                    vec![
                        TrackFragmentRandomAccessEntry::new(0, 0, 1, 1, 1),
                        TrackFragmentRandomAccessEntry::new(time, 0, largest, 1, largest),
                    ],
                );

                let payload = encoded_payload(&random_access);

                assert_eq!(payload.first(), Some(&version));
                assert_eq!(payload.get(11), Some(&(length_size << 4 | length_size)));
                assert_eq!(
                    TrackFragmentRandomAccessBox::decode_payload(&payload).unwrap(),
                    random_access
                );
            }
        }
    }

    #[test]
    fn a_box_read_with_wider_numbers_than_it_needs_is_written_back_narrower() {
        let payload = b"\0\0\0\0\0\0\0\x01\0\0\0\x3f\0\0\0\x01\
                        \0\0\0\x09\0\0\0\x08\0\0\0\x01\0\0\0\x02\0\0\0\x03";

        let random_access = TrackFragmentRandomAccessBox::decode_payload(payload).unwrap();

        assert_eq!(
            random_access,
            TrackFragmentRandomAccessBox::new(
                1,
                vec![TrackFragmentRandomAccessEntry::new(9, 8, 1, 2, 3)]
            )
        );
        assert_eq!(encoded_payload(&random_access).get(11), Some(&0));
    }

    #[test]
    fn a_box_holding_no_entries_reads_back_as_the_value_that_wrote_it() {
        let random_access = TrackFragmentRandomAccessBox::new(1, Vec::new());

        let payload = encoded_payload(&random_access);

        assert_eq!(payload, b"\0\0\0\0\0\0\0\x01\0\0\0\0\0\0\0\0");
        assert_eq!(
            TrackFragmentRandomAccessBox::decode_payload(&payload).unwrap(),
            random_access
        );
    }

    #[test]
    fn a_count_that_disagrees_with_the_entries_is_rejected() {
        let mut payload = encoded_payload(&track_fragment_random_access());
        payload
            .get_mut(12..16)
            .unwrap()
            .copy_from_slice(&3_u32.to_be_bytes());

        assert_eq!(
            TrackFragmentRandomAccessBox::decode_payload(&payload),
            Err(Error::entry_count_mismatch(3, 2))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let mut payload = encoded_payload(&track_fragment_random_access());
        *payload.first_mut().unwrap() = 2;

        assert_eq!(
            TrackFragmentRandomAccessBox::decode_payload(&payload),
            Err(Error::unsupported_version(2))
        );
    }
}
