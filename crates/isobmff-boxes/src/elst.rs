//! [`EditListBox`] (`elst`), ISO/IEC 14496-12 §8.6.6

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWidth, FieldWriter,
    FullBoxFields, FullBoxFlags,
};

/// Length of the fields that precede the entries
const FIXED_FIELDS_LEN: u64 = 8;

/// Length of one entry when version 0 carries the duration and the media time in 32 bits
const ENTRY_LEN_VERSION_0: u64 = 12;

/// Length of one entry when version 1 carries the duration and the media time in 64 bits
const ENTRY_LEN_VERSION_1: u64 = 20;

/// `media_time` of an empty edit, which maps no media onto its segment
const EMPTY_EDIT_MEDIA_TIME: i64 = -1;

/// Rate an edit plays its media at
///
/// ISO/IEC 14496-12 §8.6.6.3 has the `media_rate` take 0 or 1, written as a
/// `media_rate_integer` of that value and a `media_rate_fraction` of 0.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MediaRate {
    /// Media at the `media_time` is held for the whole segment, a dwell
    Dwell,
    /// Media plays at its own rate
    Normal,
}

/// One entry of the table an [`EditListBox`] holds
///
/// The entry is one segment of the track's timeline: it lasts
/// `segment_duration`, in the movie's time scale, and plays the media from
/// `media_time`, in the media's time scale, at `media_rate`. A `media_time` of
/// `None` is an empty edit, which plays no media for the segment.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct EditListEntry {
    segment_duration: u64,
    media_time: Option<u64>,
    media_rate: MediaRate,
}

impl EditListEntry {
    /// Creates the entry from the length of its segment and the media it plays
    #[must_use]
    pub const fn new(
        segment_duration: u64,
        media_time: Option<u64>,
        media_rate: MediaRate,
    ) -> Self {
        Self {
            segment_duration,
            media_time,
            media_rate,
        }
    }

    /// Returns the length of the segment, in the movie's time scale
    #[must_use]
    pub const fn segment_duration(&self) -> u64 {
        self.segment_duration
    }

    /// Returns the time in the media the segment starts at, in the media's time scale, or `None` for an empty edit
    #[must_use]
    pub const fn media_time(&self) -> Option<u64> {
        self.media_time
    }

    /// Returns the rate the segment plays the media at
    #[must_use]
    pub const fn media_rate(&self) -> MediaRate {
        self.media_rate
    }
}

/// Box that lists the edits laying out a track's timeline
///
/// [`EditListBox`] (`elst`), ISO/IEC 14496-12 §8.6.6. The entries lay out the
/// track's timeline segment by segment, each playing part of the media, holding
/// one point of it, or playing nothing. §8.6.6.3 has the last edit of a track
/// never be an empty one; that is not checked, in [`new`](Self::new) or in
/// [`decode_payload`](BoxDecode::decode_payload).
///
/// The version is not held: it selects how wide the `segment_duration` and
/// `media_time` of every entry are written, so
/// [`encode_payload`](BoxEncode::encode_payload) picks the narrower one
/// whenever every entry fits in 32 bits. The `entry_count` field is not held
/// either: it counts the entries, so it is derived on the way out, and on the
/// way in a count that disagrees with the entries fails the box.
#[doc(alias = "elst")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct EditListBox {
    entries: Vec<EditListEntry>,
}

impl EditListBox {
    /// Creates the box from the edits it lays the track's timeline out with
    #[must_use]
    pub const fn new(entries: Vec<EditListEntry>) -> Self {
        Self { entries }
    }

    /// Returns the entries, in the order the track's timeline runs
    #[must_use]
    pub fn entries(&self) -> &[EditListEntry] {
        &self.entries
    }

    /// Returns the length of the track's timeline the edits lay out, in the movie's time scale
    ///
    /// ISO/IEC 14496-12 §8.3.2.3 has the duration of a track with an edit list
    /// be the sum of the `segment_duration` of its edits. Returns `None` when
    /// the sum does not fit in 64 bits.
    #[must_use]
    pub fn duration(&self) -> Option<u64> {
        self.entries.iter().try_fold(0_u64, |total, entry| {
            total.checked_add(entry.segment_duration)
        })
    }

    /// Returns the version whose field width carries the durations and media times of this box
    fn version(&self) -> u8 {
        let within_32_bits = self.entries.iter().all(|entry| {
            entry.segment_duration <= u64::from(u32::MAX)
                && entry
                    .media_time
                    .is_none_or(|media_time| i32::try_from(media_time).is_ok())
        });

        if within_32_bits { 0 } else { 1 }
    }

    /// Returns the width the given version carries the durations and media times at
    const fn field_width(version: u8) -> FieldWidth {
        match version {
            0 => FieldWidth::Compact,
            _ => FieldWidth::Extended,
        }
    }
}

impl BoxDefinition for EditListBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"elst");
}

impl BoxDecode for EditListBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0 or 1.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
    ///   ends inside a field of the box or inside one of its entries.
    /// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue): an entry
    ///   states a negative `media_time` other than -1, a `media_rate_integer` other
    ///   than 0 or 1, or a `media_rate_fraction` other than 0.
    /// * [`EntryCountMismatch`](isobmff_core::ErrorKind::EntryCountMismatch): the
    ///   `entry_count` field disagrees with the entries that follow it.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version > 1 {
            return Err(Error::unsupported_version(version));
        }

        let declared = u64::from(reader.read_u32()?);
        let width = Self::field_width(version);

        let mut entries = Vec::new();
        while !reader.remainder().is_empty() {
            let segment_duration = reader.read_unsigned(width)?;
            let media_time = match reader.read_signed(width)? {
                EMPTY_EDIT_MEDIA_TIME => None,
                media_time => {
                    Some(u64::try_from(media_time).map_err(|_| Error::unsupported_value())?)
                }
            };
            let media_rate = match (reader.read_i16()?, reader.read_i16()?) {
                (0, 0) => MediaRate::Dwell,
                (1, 0) => MediaRate::Normal,
                _ => return Err(Error::unsupported_value()),
            };

            entries.push(EditListEntry {
                segment_duration,
                media_time,
                media_rate,
            });
        }

        let actual = entries.len() as u64;
        if actual != declared {
            return Err(Error::entry_count_mismatch(declared, actual));
        }

        Ok(Self { entries })
    }
}

impl BoxEncode for EditListBox {
    fn payload_len(&self) -> u64 {
        let entry_len = if self.version() == 0 {
            ENTRY_LEN_VERSION_0
        } else {
            ENTRY_LEN_VERSION_1
        };
        let entries = (self.entries.len() as u64).saturating_mul(entry_len);

        FIXED_FIELDS_LEN.saturating_add(entries)
    }

    /// # Errors
    ///
    /// * [`OutOfRange`](isobmff_core::ErrorKind::OutOfRange): an entry states a
    ///   `media_time` past what 64 signed bits hold.
    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let version = self.version();
        let width = Self::field_width(version);

        writer.write_bytes(&FullBoxFields::new(version, FullBoxFlags::ZERO).to_bytes())?;
        let entry_count = self.entries.len() as u64;
        // Why not saturate silently: an entry count past `u32` cannot be written
        // at all, and the box has already declared a length built from it, so
        // this stands for a `Vec` no target can hold.
        writer.write_unsigned(FieldWidth::Compact, entry_count)?;

        for entry in &self.entries {
            let media_time = match entry.media_time {
                None => EMPTY_EDIT_MEDIA_TIME,
                Some(media_time) => i64::try_from(media_time)
                    .map_err(|_| Error::out_of_range(media_time, FieldWidth::Extended))?,
            };
            let media_rate_integer = match entry.media_rate {
                MediaRate::Dwell => 0,
                MediaRate::Normal => 1,
            };

            writer.write_unsigned(width, entry.segment_duration)?;
            writer.write_signed(width, media_time)?;
            writer.write_i16(media_rate_integer)?;
            writer.write_i16(0)?;
        }

        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error, FieldWidth};

    use super::{EditListBox, EditListEntry, MediaRate};

    /// Edit list that starts the track 10 units into the movie and then plays its media from 0
    pub(crate) fn edit_list() -> EditListBox {
        EditListBox::new(vec![
            EditListEntry::new(10, None, MediaRate::Normal),
            EditListEntry::new(3_000, Some(0), MediaRate::Normal),
        ])
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(edit_list: &EditListBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(edit_list.payload_len()).unwrap()];
        edit_list.encode_payload(&mut buffer).unwrap();

        buffer
    }

    /// Payload of version 0 holding one entry, its media time and rate fields as given
    fn payload_of_one_entry(
        media_time: i32,
        media_rate_integer: i16,
        media_rate_fraction: i16,
    ) -> Vec<u8> {
        [
            [0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 10].as_slice(),
            &media_time.to_be_bytes(),
            &media_rate_integer.to_be_bytes(),
            &media_rate_fraction.to_be_bytes(),
        ]
        .concat()
    }

    #[test]
    fn empty_normal_and_dwell_edits_read_back_at_either_version() {
        let widest_32_bit_time = u64::try_from(i32::MAX).unwrap();
        let edit_lists = [
            (0, widest_32_bit_time, u64::from(u32::MAX)),
            (1, widest_32_bit_time + 1, u64::from(u32::MAX)),
            (1, widest_32_bit_time, u64::from(u32::MAX) + 1),
        ];

        for (version, media_time, segment_duration) in edit_lists {
            let edit_list = EditListBox::new(vec![
                EditListEntry::new(10, None, MediaRate::Normal),
                EditListEntry::new(20, Some(media_time), MediaRate::Dwell),
                EditListEntry::new(segment_duration, Some(0), MediaRate::Normal),
            ]);

            let payload = encoded_payload(&edit_list);

            assert_eq!(payload.first(), Some(&version));
            assert_eq!(EditListBox::decode_payload(&payload).unwrap(), edit_list);
        }
    }

    #[test]
    fn a_media_time_past_what_64_signed_bits_hold_is_refused() {
        let edit_list = EditListBox::new(vec![EditListEntry::new(
            0,
            Some(u64::MAX),
            MediaRate::Normal,
        )]);
        let mut buffer = vec![0; usize::try_from(edit_list.payload_len()).unwrap()];

        assert_eq!(
            edit_list.encode_payload(&mut buffer),
            Err(Error::out_of_range(u64::MAX, FieldWidth::Extended))
        );
    }

    #[test]
    fn a_rate_or_a_media_time_the_spec_does_not_give_is_rejected() {
        for payload in [
            payload_of_one_entry(0, 2, 0),
            payload_of_one_entry(0, 1, 1),
            payload_of_one_entry(-2, 1, 0),
        ] {
            assert_eq!(
                EditListBox::decode_payload(&payload),
                Err(Error::unsupported_value())
            );
        }
    }

    #[test]
    fn a_count_that_disagrees_with_the_entries_is_rejected() {
        let mut payload = encoded_payload(&edit_list());
        payload
            .get_mut(4..8)
            .unwrap()
            .copy_from_slice(&3_u32.to_be_bytes());

        assert_eq!(
            EditListBox::decode_payload(&payload),
            Err(Error::entry_count_mismatch(3, 2))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let mut payload = encoded_payload(&edit_list());
        *payload.first_mut().unwrap() = 2;

        assert_eq!(
            EditListBox::decode_payload(&payload),
            Err(Error::unsupported_version(2))
        );
    }

    #[test]
    fn the_duration_is_the_sum_of_the_segments_while_it_fits_in_64_bits() {
        let overflowing = EditListBox::new(vec![
            EditListEntry::new(u64::MAX, Some(0), MediaRate::Normal),
            EditListEntry::new(1, Some(0), MediaRate::Normal),
        ]);

        assert_eq!(edit_list().duration(), Some(3_010));
        assert_eq!(overflowing.duration(), None);
    }
}
