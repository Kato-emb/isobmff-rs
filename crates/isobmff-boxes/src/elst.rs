//! [`EditListBox`] (`elst`), ISO/IEC 14496-12 §8.6.6

use alloc::vec;
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
/// ISO/IEC 14496-12 §8.6.6.2 writes the `media_rate` as a `media_rate_integer`
/// and a `media_rate_fraction` of 0, and §8.6.6.3 has it take 0,
/// [`DWELL`](Self::DWELL), or 1, [`NORMAL`](Self::NORMAL). Those two are the
/// only rates a caller builds. Any other comes from decoding alone, as the
/// two fields the box carries.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MediaRate {
    media_rate_integer: i16,
    media_rate_fraction: i16,
}

impl MediaRate {
    /// Media at the `media_time` is held for the whole segment, a dwell
    pub const DWELL: Self = Self {
        media_rate_integer: 0,
        media_rate_fraction: 0,
    };

    /// Media plays at its own rate
    pub const NORMAL: Self = Self {
        media_rate_integer: 1,
        media_rate_fraction: 0,
    };

    /// Returns the integer part of the rate, `media_rate_integer`
    #[must_use]
    pub const fn media_rate_integer(&self) -> i16 {
        self.media_rate_integer
    }

    /// Returns the field written after the integer part, `media_rate_fraction`
    #[must_use]
    pub const fn media_rate_fraction(&self) -> i16 {
        self.media_rate_fraction
    }
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

    /// Creates the box that starts the track `starting_offset` into the movie and plays its media from `media_time` on
    ///
    /// ISO/IEC 14496-12 §8.6.6.1 represents the starting offset of a track,
    /// in the movie's time scale, by an initial empty edit that lasts it, so
    /// the box holds one when `starting_offset` is not 0. The edit that
    /// follows plays the media at [`MediaRate::NORMAL`] from `media_time`, in
    /// the media's time scale, and has a `segment_duration` of 0.
    #[must_use]
    pub fn from_starting_offset(starting_offset: u64, media_time: u64) -> Self {
        let media_edit = EditListEntry::new(0, Some(media_time), MediaRate::NORMAL);
        let entries = if starting_offset == 0 {
            vec![media_edit]
        } else {
            vec![
                EditListEntry::new(starting_offset, None, MediaRate::NORMAL),
                media_edit,
            ]
        };

        Self { entries }
    }

    /// Returns how far into the movie the track starts, in the movie's time scale
    ///
    /// Returns `Some` only for a box whose edits are at most one initial empty
    /// edit followed by one edit that plays the media at
    /// [`MediaRate::NORMAL`]. ISO/IEC 14496-12 §8.6.6.1 represents a starting
    /// offset by the initial empty edit, so the offset is its
    /// `segment_duration`, or 0 without one.
    #[must_use]
    pub fn starting_offset(&self) -> Option<u64> {
        self.starting_offset_and_media_time()
            .map(|(starting_offset, _)| starting_offset)
    }

    /// Returns the time in the media the track starts playing from, in the media's time scale
    ///
    /// Returns the `media_time` of the edit that plays the media, `Some` for
    /// the same boxes as [`starting_offset`](Self::starting_offset).
    #[must_use]
    pub fn media_time(&self) -> Option<u64> {
        self.starting_offset_and_media_time()
            .map(|(_, media_time)| media_time)
    }

    /// Returns the starting offset and the media time of a box shaped as §8.6.6.1 gives a starting offset
    fn starting_offset_and_media_time(&self) -> Option<(u64, u64)> {
        match self.entries.as_slice() {
            [
                EditListEntry {
                    media_time: Some(media_time),
                    media_rate: MediaRate::NORMAL,
                    ..
                },
            ] => Some((0, *media_time)),
            [
                EditListEntry {
                    segment_duration,
                    media_time: None,
                    ..
                },
                EditListEntry {
                    media_time: Some(media_time),
                    media_rate: MediaRate::NORMAL,
                    ..
                },
            ] => Some((*segment_duration, *media_time)),
            _ => None,
        }
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

    /// Returns the box with its last edit of length 0 lasting what `length` states from its `media_time`
    ///
    /// `length` takes the `media_time` of the last edit and returns its
    /// `segment_duration`, in the movie's time scale. Returns `None`
    /// where the last edit is not one of length 0 playing the media at
    /// [`MediaRate::NORMAL`], or where `length` returns `None`.
    pub(crate) fn with_last_edit_filled(
        &self,
        length: impl FnOnce(u64) -> Option<u64>,
    ) -> Option<Self> {
        let (last, earlier) = self.entries.split_last()?;
        let (0, Some(media_time), MediaRate::NORMAL) =
            (last.segment_duration, last.media_time, last.media_rate)
        else {
            return None;
        };
        let mut entries = earlier.to_vec();
        entries.push(EditListEntry::new(
            length(media_time)?,
            Some(media_time),
            MediaRate::NORMAL,
        ));

        Some(Self::new(entries))
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
    type Error = Error;

    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0 or 1.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
    ///   ends inside a field of the box or inside one of its entries.
    /// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue): an entry
    ///   states a negative `media_time` other than -1.
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
            let media_rate = MediaRate {
                media_rate_integer: reader.read_i16()?,
                media_rate_fraction: reader.read_i16()?,
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

            writer.write_unsigned(width, entry.segment_duration)?;
            writer.write_signed(width, media_time)?;
            writer.write_i16(entry.media_rate.media_rate_integer)?;
            writer.write_i16(entry.media_rate.media_rate_fraction)?;
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
            EditListEntry::new(10, None, MediaRate::NORMAL),
            EditListEntry::new(3_000, Some(0), MediaRate::NORMAL),
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
                EditListEntry::new(10, None, MediaRate::NORMAL),
                EditListEntry::new(20, Some(media_time), MediaRate::DWELL),
                EditListEntry::new(segment_duration, Some(0), MediaRate::NORMAL),
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
            MediaRate::NORMAL,
        )]);
        let mut buffer = vec![0; usize::try_from(edit_list.payload_len()).unwrap()];

        assert_eq!(
            edit_list.encode_payload(&mut buffer),
            Err(Error::out_of_range(u64::MAX, FieldWidth::Extended))
        );
    }

    #[test]
    fn a_rate_the_spec_does_not_give_reads_as_its_two_fields_and_back_as_the_same_value() {
        let payload = payload_of_one_entry(0, 0, 0x4000);

        let edit_list = EditListBox::decode_payload(&payload).unwrap();

        assert_eq!(
            edit_list,
            EditListBox::new(vec![EditListEntry::new(
                10,
                Some(0),
                MediaRate {
                    media_rate_integer: 0,
                    media_rate_fraction: 0x4000,
                },
            )])
        );
        assert_eq!(encoded_payload(&edit_list), payload);
    }

    #[test]
    fn a_media_time_the_spec_does_not_give_is_rejected() {
        assert_eq!(
            EditListBox::decode_payload(&payload_of_one_entry(-2, 1, 0)),
            Err(Error::unsupported_value())
        );
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
    fn a_starting_offset_and_a_media_time_read_back_through_the_box() {
        for (starting_offset, media_time) in [(0, 20), (10, 20)] {
            let payload = encoded_payload(&EditListBox::from_starting_offset(
                starting_offset,
                media_time,
            ));

            let edit_list = EditListBox::decode_payload(&payload).unwrap();

            assert_eq!(
                (edit_list.starting_offset(), edit_list.media_time()),
                (Some(starting_offset), Some(media_time))
            );
        }
    }

    #[test]
    fn a_starting_offset_is_an_initial_empty_edit_before_a_media_edit_of_no_length() {
        assert_eq!(
            EditListBox::from_starting_offset(10, 20),
            EditListBox::new(vec![
                EditListEntry::new(10, None, MediaRate::NORMAL),
                EditListEntry::new(0, Some(20), MediaRate::NORMAL),
            ])
        );
        assert_eq!(
            EditListBox::from_starting_offset(0, 20),
            EditListBox::new(vec![EditListEntry::new(0, Some(20), MediaRate::NORMAL)])
        );
    }

    #[test]
    fn an_edit_list_of_another_shape_states_no_starting_offset_or_media_time() {
        let empty_edit = EditListEntry::new(10, None, MediaRate::NORMAL);
        let media_edit = EditListEntry::new(3_000, Some(0), MediaRate::NORMAL);
        let dwell = EditListEntry::new(20, Some(0), MediaRate::DWELL);
        let edit_lists = [
            vec![],
            vec![empty_edit],
            vec![media_edit, empty_edit],
            vec![media_edit, media_edit],
            vec![empty_edit, empty_edit, media_edit],
            vec![dwell],
            vec![empty_edit, dwell],
        ];

        for entries in edit_lists {
            let edit_list = EditListBox::new(entries);

            assert_eq!(
                (edit_list.starting_offset(), edit_list.media_time()),
                (None, None)
            );
        }
    }

    #[test]
    fn the_duration_is_the_sum_of_the_segments_while_it_fits_in_64_bits() {
        let overflowing = EditListBox::new(vec![
            EditListEntry::new(u64::MAX, Some(0), MediaRate::NORMAL),
            EditListEntry::new(1, Some(0), MediaRate::NORMAL),
        ]);

        assert_eq!(edit_list().duration(), Some(3_010));
        assert_eq!(overflowing.duration(), None);
    }
}
