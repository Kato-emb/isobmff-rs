//! [`TrackRunBox`] and the [`TrackRunSample`]s it adds to a track

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxHeader,
};

use crate::data_types::SampleFlags;
use crate::full_box::{read_composition_offset, write_composition_offset};
use crate::table::{read_entries, table_len};

/// Flag mask of data-offset-present
const DATA_OFFSET_PRESENT: u32 = 0x00_0001;

/// Flag mask of first-sample-flags-present
const FIRST_SAMPLE_FLAGS_PRESENT: u32 = 0x00_0004;

/// Flag mask of sample-duration-present
const SAMPLE_DURATION_PRESENT: u32 = 0x00_0100;

/// Flag mask of sample-size-present
const SAMPLE_SIZE_PRESENT: u32 = 0x00_0200;

/// Flag mask of sample-flags-present
const SAMPLE_FLAGS_PRESENT: u32 = 0x00_0400;

/// Flag mask of sample-composition-time-offsets-present
const SAMPLE_COMPOSITION_TIME_OFFSETS_PRESENT: u32 = 0x00_0800;

/// Flags of `tr_flags` that say which fields the rows carry
const ROW_FLAGS: u32 = SAMPLE_DURATION_PRESENT
    | SAMPLE_SIZE_PRESENT
    | SAMPLE_FLAGS_PRESENT
    | SAMPLE_COMPOSITION_TIME_OFFSETS_PRESENT;

/// Flags of the low two bytes of `tr_flags` that ISO/IEC 14496-12 defines
const DEFINED_TR_FLAGS: u32 = DATA_OFFSET_PRESENT | FIRST_SAMPLE_FLAGS_PRESENT | ROW_FLAGS;

/// Sample of a track fragment run, one row of a [`TrackRunBox`]
///
/// Each field is [`Some`] where the `tr_flags` of the box say the rows carry
/// it, and [`None`] where the sample takes the default of the track fragment
/// header or of the `TrackExtendsBox`. Every row of a box carries the same
/// fields.
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct TrackRunSample {
    /// Duration of the sample, the sample-duration-present flag
    pub sample_duration: Option<u32>,
    /// Size of the sample, the sample-size-present flag
    pub sample_size: Option<u32>,
    /// Flags of the sample, the sample-flags-present flag
    pub sample_flags: Option<SampleFlags>,
    /// Offset of the composition timestamp of the sample from its decoding
    /// timestamp, the sample-composition-time-offsets-present flag
    ///
    /// Version 0 of the box carries it unsigned in 32 bits, version 1 signed
    /// in 32 bits.
    pub sample_composition_time_offset: Option<i64>,
}

impl TrackRunSample {
    /// Returns the bits of `tr_flags` that say which fields the row carries
    fn tr_flags(&self) -> u32 {
        let flag = |present: bool, mask: u32| if present { mask } else { 0 };

        flag(self.sample_duration.is_some(), SAMPLE_DURATION_PRESENT)
            | flag(self.sample_size.is_some(), SAMPLE_SIZE_PRESENT)
            | flag(self.sample_flags.is_some(), SAMPLE_FLAGS_PRESENT)
            | flag(
                self.sample_composition_time_offset.is_some(),
                SAMPLE_COMPOSITION_TIME_OFFSETS_PRESENT,
            )
    }

    /// Returns how many bytes a row carrying the fields `tr_flags` says are there takes
    fn encoded_len(tr_flags: u32) -> u64 {
        u64::from((tr_flags & ROW_FLAGS).count_ones()).saturating_mul(4)
    }

    /// Reads a row carrying the fields `tr_flags` says are there, in a box of `version`
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends inside the row.
    fn read(reader: &mut FieldReader<'_>, tr_flags: u32, version: u8) -> Result<Self, Error> {
        let mut read_if = |mask: u32| -> Result<Option<u32>, Error> {
            if tr_flags & mask == 0 {
                Ok(None)
            } else {
                reader.read_u32().map(Some)
            }
        };
        let sample_duration = read_if(SAMPLE_DURATION_PRESENT)?;
        let sample_size = read_if(SAMPLE_SIZE_PRESENT)?;
        let sample_flags = read_if(SAMPLE_FLAGS_PRESENT)?.map(SampleFlags::from_bits);
        let sample_composition_time_offset =
            if tr_flags & SAMPLE_COMPOSITION_TIME_OFFSETS_PRESENT == 0 {
                None
            } else {
                Some(read_composition_offset(reader, version)?)
            };

        Ok(Self {
            sample_duration,
            sample_size,
            sample_flags,
            sample_composition_time_offset,
        })
    }

    /// Writes the row in a box of `version`
    ///
    /// # Errors
    ///
    /// * [`OutOfRange`](isobmff_core::ErrorKind::OutOfRange): the
    ///   `sample_composition_time_offset` is outside what `version` carries.
    /// * [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue): a field of
    ///   the `sample_flags` holds a value wider than its bits.
    /// * [`TruncatedBuffer`](isobmff_core::ErrorKind::TruncatedBuffer): the
    ///   buffer ends inside the row.
    fn write(&self, writer: &mut FieldWriter<'_>, version: u8) -> Result<(), Error> {
        if let Some(sample_duration) = self.sample_duration {
            writer.write_u32(sample_duration)?;
        }
        if let Some(sample_size) = self.sample_size {
            writer.write_u32(sample_size)?;
        }
        if let Some(sample_flags) = self.sample_flags {
            writer.write_u32(sample_flags.to_bits()?)?;
        }
        if let Some(offset) = self.sample_composition_time_offset {
            write_composition_offset(writer, version, offset)?;
        }

        Ok(())
    }
}

/// Samples a [`TrackRunBox`] adds, as the rows of its table
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum TrackRunSamples {
    /// The rows are empty: every sample takes the defaults of the track
    /// fragment header or of the `TrackExtendsBox`
    EmptyRows {
        /// Number of samples the run adds
        sample_count: u32,
    },
    /// Each row carries the fields its sample does not take from the defaults
    ///
    /// Every row carries the same fields. Rows that carry none, and a run of
    /// no rows, are written as [`EmptyRows`](Self::EmptyRows) and read back as
    /// such.
    Rows {
        /// Samples the run adds, in decoding order
        samples: Vec<TrackRunSample>,
    },
}

impl Default for TrackRunSamples {
    fn default() -> Self {
        Self::EmptyRows { sample_count: 0 }
    }
}

/// [`TrackRunBox`] (`trun`), ISO/IEC 14496-12 §8.8.8
///
/// A contiguous run of samples of a track fragment. The data of the run
/// starts at `data_offset` from the base data offset of the track fragment
/// header where it is [`Some`], and otherwise right after the data of the
/// previous run, or at the base data offset for the first run of the track
/// fragment.
///
/// A flag of `tr_flags` that says a field is there is whether the field is
/// [`Some`]. ISO/IEC 14496-12 has a reader take a flag of the low two bytes it
/// does not know for a field it does not know, so a box that sets one does
/// not read: it is refused with
/// [`UnsupportedFlags`](isobmff_core::ErrorKind::UnsupportedFlags), and a
/// `TrackFragmentBox` keeps it as its bytes. A flag of the high byte is read
/// past and written as zero.
///
/// The box is written in the `version` it holds. Rows that do not all carry
/// the same fields are refused when the box is written, with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for `tr_flags`,
/// as are `first_sample_flags` together with a `sample_flags` in the rows,
/// which the specification does not allow.
///
/// The box holds `data_offset` as it was read: it counts from the base data
/// offset to the position of data in the file, which a box written back
/// elsewhere in a file no longer points at unless the writer of the file
/// adjusts it.
#[doc(alias = "trun")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct TrackRunBox {
    /// Version of the box: 0 for unsigned composition time offsets, 1 for signed ones
    pub version: u8,
    /// Offset of the data of the run from the base data offset, the data-offset-present flag
    pub data_offset: Option<i32>,
    /// Flags of the first sample of the run alone, the first-sample-flags-present flag
    pub first_sample_flags: Option<SampleFlags>,
    /// Samples the run adds
    pub samples: TrackRunSamples,
}

impl BoxDefinition for TrackRunBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"trun");
}

impl BoxDecode for TrackRunBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let header = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?);
        let version = header.version();
        if version > 1 {
            return Err(Error::unsupported_version(version));
        }
        let tr_flags = header.flags().bits();
        if tr_flags & 0x00_ffff & !DEFINED_TR_FLAGS != 0 {
            return Err(Error::unsupported_flags(tr_flags));
        }
        let sample_count = reader.read_u32()?;
        let data_offset = if tr_flags & DATA_OFFSET_PRESENT == 0 {
            None
        } else {
            Some(reader.read_i32()?)
        };
        let first_sample_flags = if tr_flags & FIRST_SAMPLE_FLAGS_PRESENT == 0 {
            None
        } else {
            Some(SampleFlags::from_bits(reader.read_u32()?))
        };
        let row_len = TrackRunSample::encoded_len(tr_flags);
        let samples = if row_len == 0 || sample_count == 0 {
            TrackRunSamples::EmptyRows { sample_count }
        } else {
            TrackRunSamples::Rows {
                samples: read_entries(reader, sample_count, row_len, |reader| {
                    TrackRunSample::read(reader, tr_flags, version)
                })?,
            }
        };

        Ok(Self {
            version,
            data_offset,
            first_sample_flags,
            samples,
        })
    }
}

impl BoxEncode for TrackRunBox {
    fn payload_len(&self) -> u64 {
        let rows_len = match &self.samples {
            TrackRunSamples::EmptyRows { .. } => 0,
            TrackRunSamples::Rows { samples } => samples.first().map_or(0, |first| {
                table_len(samples.len(), TrackRunSample::encoded_len(first.tr_flags()))
            }),
        };

        let optional_fields = [
            self.data_offset.is_some(),
            self.first_sample_flags.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count();

        table_len(optional_fields, 4)
            .saturating_add(rows_len)
            .saturating_add(4 + 4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.version > 1 {
            return Err(Error::unsupported_version(self.version));
        }
        let (sample_count, row_flags, samples) = match &self.samples {
            TrackRunSamples::EmptyRows { sample_count } => (*sample_count, 0, [].as_slice()),
            TrackRunSamples::Rows { samples } => {
                let row_flags = samples.first().map_or(0, TrackRunSample::tr_flags);
                let sample_count = u32::try_from(samples.len())
                    .map_err(|_| Error::invalid_value("sample_count"))?;
                (sample_count, row_flags, samples.as_slice())
            }
        };
        if self.first_sample_flags.is_some() && row_flags & SAMPLE_FLAGS_PRESENT != 0 {
            return Err(Error::invalid_value("tr_flags"));
        }
        let tr_flags = row_flags
            | if self.data_offset.is_some() {
                DATA_OFFSET_PRESENT
            } else {
                0
            }
            | if self.first_sample_flags.is_some() {
                FIRST_SAMPLE_FLAGS_PRESENT
            } else {
                0
            };

        writer.write_u32(u32::from(self.version) << 24 | tr_flags)?;
        writer.write_u32(sample_count)?;
        if let Some(data_offset) = self.data_offset {
            writer.write_i32(data_offset)?;
        }
        if let Some(first_sample_flags) = self.first_sample_flags {
            writer.write_u32(first_sample_flags.to_bits()?)?;
        }
        samples.iter().try_for_each(|sample| {
            if sample.tr_flags() != row_flags {
                return Err(Error::invalid_value("tr_flags"));
            }
            sample.write(writer, self.version)
        })
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error, FieldWidth};

    use super::{TrackRunBox, TrackRunSample, TrackRunSamples};
    use crate::data_types::SampleFlags;
    use crate::encoded;

    /// `trun` of version 0: data offset 0x100, sync first sample, then two samples of duration 1001, size 300 and 400, and offsets 2002 and 0
    const TRUN_V0: &[u8] = b"\0\0\0\x30trun\0\0\x0b\x05\0\0\0\x02\0\0\x01\0\0\0\0\0\
        \0\0\x03\xe9\0\0\x01\x2c\0\0\x07\xd2\
        \0\0\x03\xe9\0\0\x01\x90\0\0\0\0";

    /// `trun` of version 1: two samples of flags `0x0101_0000` and offsets -1001 and 1001
    const TRUN_V1: &[u8] = b"\0\0\0\x20trun\x01\0\x0c\0\0\0\0\x02\
        \x01\x01\0\0\xff\xff\xfc\x17\x01\x01\0\0\0\0\x03\xe9";

    /// `trun` of 25 samples that take every field from the defaults, at data offset 8
    const TRUN_EMPTY_ROWS: &[u8] = b"\0\0\0\x14trun\0\0\0\x01\0\0\0\x19\0\0\0\x08";

    fn sample(sample_size: u32, sample_composition_time_offset: i64) -> TrackRunSample {
        TrackRunSample {
            sample_duration: Some(1001),
            sample_size: Some(sample_size),
            sample_flags: None,
            sample_composition_time_offset: Some(sample_composition_time_offset),
        }
    }

    fn unsigned_offsets() -> TrackRunBox {
        TrackRunBox {
            version: 0,
            data_offset: Some(0x100),
            first_sample_flags: Some(SampleFlags::default()),
            samples: TrackRunSamples::Rows {
                samples: vec![sample(300, 2002), sample(400, 0)],
            },
        }
    }

    fn signed_offsets() -> TrackRunBox {
        let flagged = |sample_composition_time_offset| TrackRunSample {
            sample_flags: Some(SampleFlags::from_bits(0x0101_0000)),
            sample_composition_time_offset: Some(sample_composition_time_offset),
            ..TrackRunSample::default()
        };

        TrackRunBox {
            version: 1,
            data_offset: None,
            first_sample_flags: None,
            samples: TrackRunSamples::Rows {
                samples: vec![flagged(-1001), flagged(1001)],
            },
        }
    }

    fn empty_rows() -> TrackRunBox {
        TrackRunBox {
            data_offset: Some(8),
            samples: TrackRunSamples::EmptyRows { sample_count: 25 },
            ..TrackRunBox::default()
        }
    }

    #[test]
    fn reads_the_fields_and_rows_its_flags_say_are_there() {
        assert_eq!(
            (
                TrackRunBox::decode(TRUN_V0),
                TrackRunBox::decode(TRUN_V1),
                TrackRunBox::decode(TRUN_EMPTY_ROWS)
            ),
            (
                Ok((unsigned_offsets(), b"".as_slice())),
                Ok((signed_offsets(), b"".as_slice())),
                Ok((empty_rows(), b"".as_slice()))
            )
        );
    }

    #[test]
    fn writes_the_flags_of_the_fields_and_rows_that_are_there() {
        assert_eq!(
            (
                encoded(&unsigned_offsets()),
                encoded(&signed_offsets()),
                encoded(&empty_rows())
            ),
            (TRUN_V0.to_vec(), TRUN_V1.to_vec(), TRUN_EMPTY_ROWS.to_vec())
        );
    }

    #[test]
    fn rows_that_carry_no_field_are_written_as_empty_rows() {
        let mut rows = empty_rows();
        rows.samples = TrackRunSamples::Rows {
            samples: vec![TrackRunSample::default(); 25],
        };

        assert_eq!(encoded(&rows), TRUN_EMPTY_ROWS);
    }

    #[test]
    fn a_run_of_no_samples_is_read_as_empty_rows_whatever_fields_its_flags_put_in_a_row() {
        assert_eq!(
            TrackRunBox::decode(b"\0\0\0\x18trun\0\0\x02\x01\0\0\0\0\0\0ett\0ef"),
            Ok((
                TrackRunBox {
                    data_offset: Some(0x6574),
                    samples: TrackRunSamples::EmptyRows { sample_count: 0 },
                    ..TrackRunBox::default()
                },
                b"".as_slice()
            ))
        );
    }

    #[test]
    fn a_flag_of_the_high_byte_is_read_past_and_written_as_zero() {
        let (read, _) =
            TrackRunBox::decode(b"\0\0\0\x14trun\0\x80\0\x01\0\0\0\x19\0\0\0\x08").unwrap();

        assert_eq!(encoded(&read), TRUN_EMPTY_ROWS);
    }

    #[test]
    fn a_flag_of_the_low_bytes_the_specification_does_not_define_is_refused() {
        assert_eq!(
            TrackRunBox::decode_payload(b"\0\0\x10\x01\0\0\0\x19\0\0\0\x08"),
            Err(Error::unsupported_flags(0x1001))
        );
    }

    #[test]
    fn rows_that_do_not_carry_the_same_fields_are_refused_when_written() {
        let mut ragged = signed_offsets();
        ragged.samples = TrackRunSamples::Rows {
            samples: vec![sample(300, 0), TrackRunSample::default()],
        };

        assert_eq!(
            ragged.encode(&mut [0; 64]),
            Err(Error::invalid_value("tr_flags"))
        );
    }

    #[test]
    fn first_sample_flags_with_sample_flags_in_the_rows_are_refused_when_written() {
        let mut both = signed_offsets();
        both.first_sample_flags = Some(SampleFlags::default());

        assert_eq!(
            both.encode(&mut [0; 64]),
            Err(Error::invalid_value("tr_flags"))
        );
    }

    #[test]
    fn version_0_refuses_a_negative_composition_time_offset() {
        let mut negative = signed_offsets();
        negative.version = 0;

        assert_eq!(
            negative.encode(&mut [0; 64]),
            Err(Error::out_of_range(
                (-1001_i64).cast_unsigned(),
                FieldWidth::Compact
            ))
        );
    }

    #[test]
    fn a_payload_ending_inside_a_row_is_refused() {
        assert_eq!(
            TrackRunBox::decode_payload(TRUN_V0.get(8..46).unwrap()),
            Err(Error::truncated_payload(40, 38))
        );
    }

    #[test]
    fn a_count_of_more_rows_than_the_payload_holds_is_refused() {
        assert_eq!(
            TrackRunBox::decode_payload(b"\0\0\x01\0\xff\xff\xff\xff\0\0\x03\xe9"),
            Err(Error::truncated_payload(8 + 0xffff_ffff * 4, 12))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused_when_read_and_when_written() {
        let mut version_2 = empty_rows();
        version_2.version = 2;

        assert_eq!(
            (
                TrackRunBox::decode_payload(b"\x02\0\0\0\0\0\0\0"),
                version_2.encode(&mut [0; 32])
            ),
            (
                Err(Error::unsupported_version(2)),
                Err(Error::unsupported_version(2))
            )
        );
    }
}
