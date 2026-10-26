//! [`TrackFragmentRandomAccessBox`] and the [`TrackFragmentRandomAccessEntry`]s it holds

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFlags,
    FullBoxHeader,
};

use crate::full_box::version_width;
use crate::table::{read_entries, table_len, write_entry_count};

/// Location and presentation time of a sync sample, one entry of a [`TrackFragmentRandomAccessBox`]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct TrackFragmentRandomAccessEntry {
    /// Presentation time of the sync sample, in the timescale of the `MediaHeaderBox` of the track
    pub time: u64,
    /// Offset from the beginning of the file to the beginning of the `moof` that holds the sync sample
    pub moof_offset: u64,
    /// Number of the `traf` that holds the sync sample within its `moof`, from 1
    pub traf_number: u32,
    /// Number of the `trun` that holds the sync sample within its `traf`, from 1
    pub trun_number: u32,
    /// One plus the number of the sync sample minus that of the first sample of its `trun`
    pub sample_delta: u32,
}

/// [`TrackFragmentRandomAccessBox`] (`tfra`), ISO/IEC 14496-12 §8.8.10
///
/// The location and presentation time of sync samples of one track in its
/// movie fragments. Not every sync sample need be listed; a box of no entry
/// says every sample is a sync sample.
///
/// The box is written in the `version` it holds, `time` and `moof_offset` in
/// 32 bits under version 0 and in 64 under version 1, and with the
/// `traf_number`, `trun_number` and `sample_delta` of every entry in as many
/// bytes as the `length_size_of_*` fields say, one more than each. A value
/// that does not fit its width is refused when the box is written, with
/// [`OutOfRange`](isobmff_core::ErrorKind::OutOfRange) for `time` and
/// `moof_offset` and with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for the others,
/// as is a `length_size_of_*` above 3.
///
/// The box holds `moof_offset` as it was read: it is the position of a box
/// in the file, which a writer of the file adjusts when the boxes move.
#[doc(alias = "tfra")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct TrackFragmentRandomAccessBox {
    /// Version of the box, 0 or 1, which sets the width of `time` and `moof_offset`
    pub version: u8,
    /// `track_ID` of the track the entries are for
    pub track_id: u32,
    /// Length in bytes of `traf_number`, minus one
    pub length_size_of_traf_num: u8,
    /// Length in bytes of `trun_number`, minus one
    pub length_size_of_trun_num: u8,
    /// Length in bytes of `sample_delta`, minus one
    pub length_size_of_sample_num: u8,
    /// Sync samples, in the order they came
    pub entries: Vec<TrackFragmentRandomAccessEntry>,
}

impl TrackFragmentRandomAccessBox {
    /// Returns the bytes each of `traf_number`, `trun_number` and `sample_delta` takes
    fn number_lens(&self) -> [u8; 3] {
        [
            self.length_size_of_traf_num,
            self.length_size_of_trun_num,
            self.length_size_of_sample_num,
        ]
        .map(|length_size| length_size.saturating_add(1))
    }

    /// Returns the bytes each entry takes in the version and widths the box holds
    fn entry_len(&self) -> u64 {
        self.number_lens()
            .iter()
            .map(|len| u64::from(*len))
            .fold(if self.version == 1 { 16 } else { 8 }, u64::saturating_add)
    }
}

impl BoxDefinition for TrackFragmentRandomAccessBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tfra");
}

impl BoxDecode for TrackFragmentRandomAccessBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?).version();
        let width = version_width(version)?;
        let track_id = reader.read_u32()?;
        let [.., length_sizes] = reader.read_u32()?.to_be_bytes();
        let mut read = Self {
            version,
            track_id,
            length_size_of_traf_num: length_sizes >> 4 & 0b11,
            length_size_of_trun_num: length_sizes >> 2 & 0b11,
            length_size_of_sample_num: length_sizes & 0b11,
            entries: Vec::new(),
        };
        let [traf_len, trun_len, sample_len] = read.number_lens();
        let number_of_entry = reader.read_u32()?;
        read.entries = read_entries(reader, number_of_entry, read.entry_len(), |reader| {
            let time = reader.read_unsigned(width)?;
            let moof_offset = reader.read_unsigned(width)?;
            let mut read_number = |len: u8| {
                reader.read_slice(usize::from(len)).map(|bytes| {
                    bytes
                        .iter()
                        .fold(0_u32, |number, byte| number << 8 | u32::from(*byte))
                })
            };

            Ok(TrackFragmentRandomAccessEntry {
                time,
                moof_offset,
                traf_number: read_number(traf_len)?,
                trun_number: read_number(trun_len)?,
                sample_delta: read_number(sample_len)?,
            })
        })?;

        Ok(read)
    }
}

impl BoxEncode for TrackFragmentRandomAccessBox {
    fn payload_len(&self) -> u64 {
        table_len(self.entries.len(), self.entry_len()).saturating_add(4 + 4 + 4 + 4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let width = version_width(self.version)?;
        let length_sizes = [
            (self.length_size_of_traf_num, "length_size_of_traf_num"),
            (self.length_size_of_trun_num, "length_size_of_trun_num"),
            (self.length_size_of_sample_num, "length_size_of_sample_num"),
        ];
        if let Some((_, field)) = length_sizes.iter().find(|(value, _)| *value > 0b11) {
            return Err(Error::invalid_value(field));
        }
        let number_lens = self.number_lens();

        writer.write_bytes(&FullBoxHeader::new(self.version, FullBoxFlags::ZERO).to_bytes())?;
        writer.write_u32(self.track_id)?;
        writer.write_u32(u32::from(
            self.length_size_of_traf_num << 4
                | self.length_size_of_trun_num << 2
                | self.length_size_of_sample_num,
        ))?;
        write_entry_count(writer, self.entries.len())?;
        for entry in &self.entries {
            writer.write_unsigned(width, entry.time)?;
            writer.write_unsigned(width, entry.moof_offset)?;
            let numbers = [
                (entry.traf_number, "traf_number"),
                (entry.trun_number, "trun_number"),
                (entry.sample_delta, "sample_delta"),
            ];
            for ((number, field), len) in numbers.into_iter().zip(number_lens) {
                let bytes = number.to_be_bytes();
                let (high, low) = bytes.split_at(bytes.len().saturating_sub(usize::from(len)));
                if high.iter().any(|byte| *byte != 0) {
                    return Err(Error::invalid_value(field));
                }
                writer.write_slice(low)?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error, FieldWidth};

    use super::{TrackFragmentRandomAccessBox, TrackFragmentRandomAccessEntry};
    use crate::encoded;

    /// `tfra` of version 0, track 1, 1-byte `traf_number`, 2-byte `trun_number` and 4-byte `sample_delta`: sync samples at 0 in the `moof` at 0x30 and at 3000 in the `moof` at 0x1000
    const TFRA_V0: &[u8] = b"\0\0\0\x36tfra\0\0\0\0\0\0\0\x01\0\0\0\x07\0\0\0\x02\
        \0\0\0\0\0\0\0\x30\x01\0\x01\0\0\0\x01\
        \0\0\x0b\xb8\0\0\x10\0\x01\0\x02\0\0\0\x05";

    /// `tfra` of version 1, track 2, 1-byte numbers: a sync sample at 2^32 in the `moof` at 2^33
    const TFRA_V1: &[u8] = b"\0\0\0\x2btfra\x01\0\0\0\0\0\0\x02\0\0\0\0\0\0\0\x01\
        \0\0\0\x01\0\0\0\0\0\0\0\x02\0\0\0\0\x01\x01\x01";

    fn entry(
        time: u64,
        moof_offset: u64,
        trun_number: u32,
        sample_delta: u32,
    ) -> TrackFragmentRandomAccessEntry {
        TrackFragmentRandomAccessEntry {
            time,
            moof_offset,
            traf_number: 1,
            trun_number,
            sample_delta,
        }
    }

    fn compact() -> TrackFragmentRandomAccessBox {
        TrackFragmentRandomAccessBox {
            version: 0,
            track_id: 1,
            length_size_of_traf_num: 0,
            length_size_of_trun_num: 1,
            length_size_of_sample_num: 3,
            entries: vec![entry(0, 0x30, 1, 1), entry(3000, 0x1000, 2, 5)],
        }
    }

    fn extended() -> TrackFragmentRandomAccessBox {
        TrackFragmentRandomAccessBox {
            version: 1,
            track_id: 2,
            entries: vec![entry(1 << 32, 1 << 33, 1, 1)],
            ..TrackFragmentRandomAccessBox::default()
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_in_either_version() {
        assert_eq!(
            (
                TrackFragmentRandomAccessBox::decode(TFRA_V0),
                TrackFragmentRandomAccessBox::decode(TFRA_V1)
            ),
            (
                Ok((compact(), b"".as_slice())),
                Ok((extended(), b"".as_slice()))
            )
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_the_version_and_widths_it_holds() {
        assert_eq!(
            (encoded(&compact()), encoded(&extended())),
            (TFRA_V0.to_vec(), TFRA_V1.to_vec())
        );
    }

    #[test]
    fn the_reserved_bits_are_read_past_and_written_as_zero() {
        let mut reserved_set = TFRA_V0.to_vec();
        if let Some(byte) = reserved_set.get_mut(16) {
            *byte = 0xff;
        }
        let (read, _) = TrackFragmentRandomAccessBox::decode(&reserved_set).unwrap();

        assert_eq!(encoded(&read), TFRA_V0);
    }

    #[test]
    fn values_that_do_not_fit_their_widths_are_refused_when_written() {
        let mut wide_number = compact();
        wide_number.entries = vec![entry(0, 0, 0x1_0000, 1)];
        let mut wide_length_size = compact();
        wide_length_size.length_size_of_sample_num = 4;
        let mut wide_time = compact();
        wide_time.entries = vec![entry(1 << 32, 0, 1, 1)];

        assert_eq!(
            (
                wide_number.encode(&mut [0; 64]),
                wide_length_size.encode(&mut [0; 64]),
                wide_time.encode(&mut [0; 64])
            ),
            (
                Err(Error::invalid_value("trun_number")),
                Err(Error::invalid_value("length_size_of_sample_num")),
                Err(Error::out_of_range(1 << 32, FieldWidth::Compact))
            )
        );
    }

    #[test]
    fn a_count_of_more_entries_than_the_payload_holds_is_refused() {
        assert_eq!(
            TrackFragmentRandomAccessBox::decode_payload(TFRA_V0.get(8..30).unwrap()),
            Err(Error::truncated_payload(16 + 2 * 15, 22))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            TrackFragmentRandomAccessBox::decode_payload(b"\x02\0\0\0"),
            Err(Error::unsupported_version(2))
        );
    }
}
