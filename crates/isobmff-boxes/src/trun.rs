//! [`TrackRunBox`] (`trun`), ISO/IEC 14496-12 §8.8.8

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, DecodeError, EncodeError, FieldReadError,
    FieldReader, FieldWriter, FullBoxFields, FullBoxFlags,
};

/// Length of the fields that precede the optional ones
const FIXED_FIELDS_LEN: u64 = 8;

/// Length of every optional field, whether it precedes the rows or lies in one
const OPTIONAL_FIELD_LEN: u64 = 4;

/// Flag stating that `data_offset` is present
const DATA_OFFSET_PRESENT: u32 = 0x0000_0001;

/// Flag stating that `first_sample_flags` is present
const FIRST_SAMPLE_FLAGS_PRESENT: u32 = 0x0000_0004;

/// Flag stating that every row carries a `sample_duration`
const SAMPLE_DURATION_PRESENT: u32 = 0x0000_0100;

/// Flag stating that every row carries a `sample_size`
const SAMPLE_SIZE_PRESENT: u32 = 0x0000_0200;

/// Flag stating that every row carries its own `sample_flags`
const SAMPLE_FLAGS_PRESENT: u32 = 0x0000_0400;

/// Flag stating that every row carries a `sample_composition_time_offset`
const SAMPLE_COMPOSITION_TIME_OFFSETS_PRESENT: u32 = 0x0000_0800;

/// Every flag stating that a field of this box lies in each of its rows
const PER_SAMPLE_FLAGS: u32 = SAMPLE_DURATION_PRESENT
    | SAMPLE_SIZE_PRESENT
    | SAMPLE_FLAGS_PRESENT
    | SAMPLE_COMPOSITION_TIME_OFFSETS_PRESENT;

/// Every flag this box reads
const DEFINED_FLAGS: u32 = DATA_OFFSET_PRESENT | FIRST_SAMPLE_FLAGS_PRESENT | PER_SAMPLE_FLAGS;

/// Rows this box reads from a run whose rows are empty
const MAXIMUM_EMPTY_ROWS: u64 = 1 << 20;

/// Widest composition time offset a row carries, which version 0 writes unsigned
const COMPOSITION_TIME_OFFSET_MAXIMUM: i64 = u32::MAX as i64;

/// Lowest composition time offset a row carries, which version 1 writes signed
const COMPOSITION_TIME_OFFSET_MINIMUM: i64 = i32::MIN as i64;

/// One row of the table a track run documents, holding what it states per sample
///
/// Which fields a row carries is stated once for the whole run, so every row of
/// one [`TrackRunBox`] carries the same ones, and a field no row carries falls
/// back on the default the `tfhd` or the `trex` sets.
///
/// The composition time offset is held signed and wide enough for either version
/// of the box to write it: version 0 carries it as an unsigned 32-bit field and
/// version 1 as a signed one, so [`new`](Self::new) refuses a value neither can
/// carry.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TrackRunSample {
    sample_duration: Option<u32>,
    sample_size: Option<u32>,
    sample_flags: Option<u32>,
    sample_composition_time_offset: Option<i64>,
}

impl TrackRunSample {
    /// Creates one row from the fields the run states for its sample
    ///
    /// Returns `None` when `sample_composition_time_offset` lies outside
    /// `-2_147_483_648..=4_294_967_295`, the offsets the two versions of the box
    /// write between them.
    #[must_use]
    pub fn new(
        sample_duration: Option<u32>,
        sample_size: Option<u32>,
        sample_flags: Option<u32>,
        sample_composition_time_offset: Option<i64>,
    ) -> Option<Self> {
        let carried = COMPOSITION_TIME_OFFSET_MINIMUM..=COMPOSITION_TIME_OFFSET_MAXIMUM;
        if sample_composition_time_offset.is_some_and(|offset| !carried.contains(&offset)) {
            return None;
        }

        Some(Self {
            sample_duration,
            sample_size,
            sample_flags,
            sample_composition_time_offset,
        })
    }

    /// Returns how long this sample lasts, in the media time scale
    #[must_use]
    pub const fn sample_duration(&self) -> Option<u32> {
        self.sample_duration
    }

    /// Returns how many bytes this sample occupies
    #[must_use]
    pub const fn sample_size(&self) -> Option<u32> {
        self.sample_size
    }

    /// Returns the flags of this sample, which state how it may be decoded
    #[must_use]
    pub const fn sample_flags(&self) -> Option<u32> {
        self.sample_flags
    }

    /// Returns the offset from the decode time of this sample to its composition time
    #[must_use]
    pub const fn sample_composition_time_offset(&self) -> Option<i64> {
        self.sample_composition_time_offset
    }
}

/// Returns a length of bytes as the counts of this module are held
fn byte_count(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}

/// Returns the flags stating which of the per-sample fields one row carries
fn carried_field_flags(sample: &TrackRunSample) -> u32 {
    sample
        .sample_duration
        .map_or(0, |_| SAMPLE_DURATION_PRESENT)
        | sample.sample_size.map_or(0, |_| SAMPLE_SIZE_PRESENT)
        | sample.sample_flags.map_or(0, |_| SAMPLE_FLAGS_PRESENT)
        | sample
            .sample_composition_time_offset
            .map_or(0, |_| SAMPLE_COMPOSITION_TIME_OFFSETS_PRESENT)
}

/// Returns the flags stating which of the per-sample fields every row of a run carries
fn per_sample_field_flags(samples: &[TrackRunSample]) -> u32 {
    samples.first().map_or(0, carried_field_flags)
}

/// Box that documents a contiguous run of the samples of one track fragment
///
/// [`TrackRunBox`] (`trun`), ISO/IEC 14496-12 §8.8.8. The run states, per
/// sample, whatever its samples do not share — [`TrackRunSample`] is one row of
/// that table — and leaves the rest to the defaults the `tfhd` and the `trex`
/// set. A `traf` carries as many runs as it has contiguous runs of samples.
///
/// The `sample_count` is not held: it counts the rows, so it is derived on the
/// way out. The `flags` are not held either — every flag this box reads states
/// that one of the optional fields is present, which the fields themselves
/// already say. A flag this box does not read is refused rather than carried
/// through.
///
/// A run that states no per-sample field still counts its samples, and §8.8.8
/// allows those rows to be empty — which leaves the payload no say in how many
/// there are. [`decode_payload`](BoxDecode::decode_payload) reads up to
/// `1_048_576` such rows and refuses a count past that.
///
/// The version is not held: it selects whether the composition time offsets are
/// written signed or unsigned, so
/// [`encode_payload`](BoxEncode::encode_payload) writes version 0 unless a row
/// carries a negative offset.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{TrackRunBox, TrackRunSample};
/// use isobmff_core::BoxWrite;
///
/// // Two samples, each stating its own size and nothing else
/// let samples = vec![
///     TrackRunSample::new(None, Some(1_024), None, None).unwrap(),
///     TrackRunSample::new(None, Some(2_048), None, None).unwrap(),
/// ];
/// let track_run = TrackRunBox::new(Some(0), None, samples).unwrap();
///
/// // The box header, the count, the data offset, and one field per sample
/// assert_eq!(track_run.encoded_len(), 28);
///
/// // A row carrying fields the others do not builds nothing
/// assert_eq!(
///     TrackRunBox::new(
///         None,
///         None,
///         vec![
///             TrackRunSample::new(None, Some(1_024), None, None).unwrap(),
///             TrackRunSample::new(Some(512), Some(2_048), None, None).unwrap(),
///         ]
///     ),
///     None
/// );
/// ```
#[doc(alias = "trun")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TrackRunBox {
    data_offset: Option<i32>,
    first_sample_flags: Option<u32>,
    samples: Vec<TrackRunSample>,
}

impl TrackRunBox {
    /// Creates the box from the run of samples it documents
    ///
    /// Returns `None` when
    ///
    /// * the rows do not all carry the same fields, which the flags state once
    ///   for the whole run;
    /// * `first_sample_flags` is given while the rows carry flags of their own,
    ///   which §8.8.8 forbids together;
    /// * one row carries a negative composition time offset while another
    ///   carries one past [`i32::MAX`], which leaves no version able to write
    ///   both.
    #[must_use]
    pub fn new(
        data_offset: Option<i32>,
        first_sample_flags: Option<u32>,
        samples: Vec<TrackRunSample>,
    ) -> Option<Self> {
        let carried = per_sample_field_flags(&samples);
        if samples
            .iter()
            .any(|sample| carried_field_flags(sample) != carried)
        {
            return None;
        }
        if first_sample_flags.is_some() && carried & SAMPLE_FLAGS_PRESENT != 0 {
            return None;
        }

        let offsets = || {
            samples
                .iter()
                .filter_map(TrackRunSample::sample_composition_time_offset)
        };
        let signed = offsets().any(i64::is_negative);
        let past_the_signed_range = offsets().any(|offset| offset > i64::from(i32::MAX));
        if signed && past_the_signed_range {
            return None;
        }

        Some(Self {
            data_offset,
            first_sample_flags,
            samples,
        })
    }

    /// Returns the offset this run counts from the one the `tfhd` established
    #[must_use]
    pub const fn data_offset(&self) -> Option<i32> {
        self.data_offset
    }

    /// Returns the flags of the first sample of the run, which override the defaults
    #[must_use]
    pub const fn first_sample_flags(&self) -> Option<u32> {
        self.first_sample_flags
    }

    /// Returns the samples of the run, in the order they are decoded
    #[must_use]
    pub fn samples(&self) -> &[TrackRunSample] {
        &self.samples
    }
}

impl BoxDefinition for TrackRunBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"trun");
}

impl BoxDecode for TrackRunBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](DecodeError::UnsupportedVersion): the box
    ///   declares a version other than 0 or 1.
    /// * [`UnsupportedFlags`](DecodeError::UnsupportedFlags): the box declares a
    ///   flag this box does not read, which stands for a field it cannot place.
    /// * [`ConflictingFlags`](DecodeError::ConflictingFlags): the box states the
    ///   flags of its first sample and of every sample at once.
    /// * [`UnsupportedEntryCount`](DecodeError::UnsupportedEntryCount): the rows
    ///   are empty and the `sample_count` is past the rows this box reads.
    /// * [`Field`](DecodeError::Field): the payload ends inside a field the flags
    ///   state, or holds bytes past the rows the `sample_count` declares.
    fn decode_payload(payload: &[u8]) -> Result<Self, DecodeError> {
        let mut reader = FieldReader::new(payload);
        let full_box = FullBoxFields::from_bytes(reader.read_bytes::<4>()?);
        let version = full_box.version();
        if version > 1 {
            return Err(DecodeError::UnsupportedVersion(version));
        }

        let flags = full_box.flags().bits();
        // Why not carrying an undefined bit through, as the `tfhd` does: §8.8.8
        // has the length of a row follow the bits set in the flags, so a bit this
        // box does not read stands for a field it cannot place in a row.
        let undefined = flags & !DEFINED_FLAGS;
        if undefined != 0 {
            return Err(DecodeError::UnsupportedFlags(undefined));
        }
        let carries = |flag: u32| flags & flag != 0;
        if carries(FIRST_SAMPLE_FLAGS_PRESENT) && carries(SAMPLE_FLAGS_PRESENT) {
            return Err(DecodeError::ConflictingFlags(
                FIRST_SAMPLE_FLAGS_PRESENT | SAMPLE_FLAGS_PRESENT,
            ));
        }

        let sample_count = reader.read_u32()?;
        let data_offset = if carries(DATA_OFFSET_PRESENT) {
            Some(reader.read_i32()?)
        } else {
            None
        };
        let first_sample_flags = if carries(FIRST_SAMPLE_FLAGS_PRESENT) {
            Some(reader.read_u32()?)
        } else {
            None
        };

        let row_len =
            u64::from((flags & PER_SAMPLE_FLAGS).count_ones()).saturating_mul(OPTIONAL_FIELD_LEN);
        let rows_len = row_len.saturating_mul(u64::from(sample_count));
        let remaining = byte_count(reader.remainder().len());
        // Why not reading the rows and letting the reader report the shortfall:
        // the count comes from the input, so a row length of four bytes lets a
        // twelve-byte payload declare four billion rows, and the reading would
        // hold a gigabyte of them before the payload ran out.
        if rows_len > remaining {
            let available = byte_count(payload.len());

            return Err(DecodeError::Field(FieldReadError::UnexpectedEof {
                needed: available.saturating_sub(remaining).saturating_add(rows_len),
                available,
            }));
        }

        let declared = u64::from(sample_count);
        if row_len == 0 && declared > MAXIMUM_EMPTY_ROWS {
            return Err(DecodeError::UnsupportedEntryCount {
                declared,
                limit: MAXIMUM_EMPTY_ROWS,
            });
        }

        // Why not with_capacity: the count is bounded above, by the payload for
        // rows that occupy bytes and by the limit for rows that do not, but
        // reserving still hands a four-byte field the whole bound up front.
        let mut samples = Vec::new();
        for _ in 0..sample_count {
            let sample_duration = if carries(SAMPLE_DURATION_PRESENT) {
                Some(reader.read_u32()?)
            } else {
                None
            };
            let sample_size = if carries(SAMPLE_SIZE_PRESENT) {
                Some(reader.read_u32()?)
            } else {
                None
            };
            let sample_flags = if carries(SAMPLE_FLAGS_PRESENT) {
                Some(reader.read_u32()?)
            } else {
                None
            };
            let sample_composition_time_offset = if carries(SAMPLE_COMPOSITION_TIME_OFFSETS_PRESENT)
            {
                Some(match version {
                    0 => i64::from(reader.read_u32()?),
                    _ => i64::from(reader.read_i32()?),
                })
            } else {
                None
            };

            samples.push(TrackRunSample {
                sample_duration,
                sample_size,
                sample_flags,
                sample_composition_time_offset,
            });
        }
        reader.finish()?;

        Ok(Self {
            data_offset,
            first_sample_flags,
            samples,
        })
    }
}

impl BoxEncode for TrackRunBox {
    fn payload_len(&self) -> u64 {
        let length = FIXED_FIELDS_LEN
            .saturating_add(self.data_offset.map_or(0, |_| OPTIONAL_FIELD_LEN))
            .saturating_add(self.first_sample_flags.map_or(0, |_| OPTIONAL_FIELD_LEN));

        let row = u64::from(per_sample_field_flags(&self.samples).count_ones())
            .saturating_mul(OPTIONAL_FIELD_LEN);
        let rows = row.saturating_mul(u64::try_from(self.samples.len()).unwrap_or(u64::MAX));

        length.saturating_add(rows)
    }

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), EncodeError> {
        let expected = self.payload_len();
        let actual = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        let mismatch = EncodeError::BufferLengthMismatch { expected, actual };
        if actual != expected {
            return Err(mismatch);
        }

        let bits = per_sample_field_flags(&self.samples)
            | self.data_offset.map_or(0, |_| DATA_OFFSET_PRESENT)
            | self
                .first_sample_flags
                .map_or(0, |_| FIRST_SAMPLE_FLAGS_PRESENT);

        let signed = self
            .samples
            .iter()
            .filter_map(TrackRunSample::sample_composition_time_offset)
            .any(i64::is_negative);
        // Why not version 1 throughout: §8.6.1.3 asks for the unsigned form
        // wherever it carries the offsets, which the readers of earlier brands
        // accept.
        let version = if signed { 1 } else { 0 };

        // Why not unwrap: the bits are the flags this box defines, which lie
        // inside the field by construction, and the payload traits allow a
        // failure that can no longer happen to be reported as the mismatch.
        let flags = FullBoxFlags::new(bits).ok_or(mismatch)?;
        let mut writer = FieldWriter::new(buffer);

        writer.write_bytes(&FullBoxFields::new(version, flags).to_bytes())?;
        // Why not saturate silently: a row count past `u32` cannot be written at
        // all, and the box has already declared a length built from it, so this
        // stands for a `Vec` no target can hold.
        writer.write_u32(u32::try_from(self.samples.len()).map_err(|_| mismatch)?)?;
        if let Some(data_offset) = self.data_offset {
            writer.write_i32(data_offset)?;
        }
        if let Some(first_sample_flags) = self.first_sample_flags {
            writer.write_u32(first_sample_flags)?;
        }

        for sample in &self.samples {
            for field in [
                sample.sample_duration,
                sample.sample_size,
                sample.sample_flags,
            ]
            .into_iter()
            .flatten()
            {
                writer.write_u32(field)?;
            }
            if let Some(offset) = sample.sample_composition_time_offset {
                if version == 0 {
                    writer.write_u32(u32::try_from(offset).map_err(|_| mismatch)?)?;
                } else {
                    writer.write_i32(i32::try_from(offset).map_err(|_| mismatch)?)?;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, DecodeError, FieldReadError};

    use super::{MAXIMUM_EMPTY_ROWS, TrackRunBox, TrackRunSample};

    /// Row stating the size of its sample and the offset to its composition time
    fn sample(sample_size: u32, sample_composition_time_offset: i64) -> TrackRunSample {
        TrackRunSample::new(
            None,
            Some(sample_size),
            None,
            Some(sample_composition_time_offset),
        )
        .unwrap()
    }

    /// Run of two samples anchored at the start of the data of its fragment
    fn track_run() -> TrackRunBox {
        TrackRunBox::new(Some(0), None, vec![sample(1_024, 0), sample(2_048, 512)]).unwrap()
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(track_run: &TrackRunBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(track_run.payload_len()).unwrap()];
        track_run.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let payload = encoded_payload(&track_run());

        assert_eq!(TrackRunBox::decode_payload(&payload).unwrap(), track_run());
    }

    #[test]
    fn the_sample_count_is_written_from_the_rows_the_box_holds() {
        let payload = encoded_payload(&track_run());

        assert_eq!(payload.get(4..8), Some(b"\0\0\0\x02".as_slice()));
    }

    #[test]
    fn the_flags_state_which_fields_the_run_and_its_rows_carry() {
        let payload = encoded_payload(&track_run());

        assert_eq!(payload.get(..4), Some(b"\0\0\x0a\x01".as_slice()));
    }

    #[test]
    fn a_run_holding_no_samples_declares_a_count_of_zero() {
        let payload = encoded_payload(&TrackRunBox::new(None, None, Vec::new()).unwrap());

        assert_eq!(payload, b"\0\0\0\0\0\0\0\0");
    }

    #[test]
    fn an_offset_past_the_signed_range_is_written_unsigned_at_version_0() {
        let run = TrackRunBox::new(None, None, vec![sample(1_024, 3_000_000_000)]).unwrap();

        let payload = encoded_payload(&run);

        assert_eq!(payload.first(), Some(&0));
        assert_eq!(TrackRunBox::decode_payload(&payload).unwrap(), run);
    }

    #[test]
    fn a_negative_offset_moves_the_offsets_of_the_run_to_version_1() {
        let run = TrackRunBox::new(None, None, vec![sample(1_024, -512)]).unwrap();

        let payload = encoded_payload(&run);

        assert_eq!(payload.first(), Some(&1));
        assert_eq!(TrackRunBox::decode_payload(&payload).unwrap(), run);
    }

    #[test]
    fn a_row_carrying_an_offset_no_version_writes_cannot_be_built() {
        assert_eq!(
            TrackRunSample::new(None, None, None, Some(i64::from(u32::MAX) + 1)),
            None
        );
        assert_eq!(
            TrackRunSample::new(None, None, None, Some(i64::from(i32::MIN) - 1)),
            None
        );
    }

    #[test]
    fn a_run_mixing_a_negative_offset_with_one_past_the_signed_range_cannot_be_built() {
        assert_eq!(
            TrackRunBox::new(
                None,
                None,
                vec![sample(1_024, -512), sample(2_048, 3_000_000_000)]
            ),
            None
        );
    }

    #[test]
    fn a_run_whose_rows_carry_different_fields_cannot_be_built() {
        assert_eq!(
            TrackRunBox::new(
                None,
                None,
                vec![
                    sample(1_024, 0),
                    TrackRunSample::new(None, Some(2_048), None, None).unwrap()
                ]
            ),
            None
        );
    }

    #[test]
    fn a_run_stating_the_flags_of_its_first_sample_and_of_every_sample_cannot_be_built() {
        let flagged = TrackRunSample::new(None, None, Some(0x0100_0000), None).unwrap();

        assert_eq!(
            TrackRunBox::new(None, Some(0x0200_0000), vec![flagged]),
            None
        );
    }

    #[test]
    fn a_payload_stating_the_flags_of_its_first_sample_and_of_every_sample_is_rejected() {
        let payload = b"\0\0\x04\x04\0\0\0\x01\0\0\0\0\0\0\0\0";

        assert!(matches!(
            TrackRunBox::decode_payload(payload),
            Err(DecodeError::ConflictingFlags(0x0000_0404))
        ));
    }

    #[test]
    fn a_flag_the_box_does_not_read_is_rejected() {
        let payload = b"\0\0\x10\0\0\0\0\0";

        assert!(matches!(
            TrackRunBox::decode_payload(payload),
            Err(DecodeError::UnsupportedFlags(0x0000_1000))
        ));
    }

    #[test]
    fn a_declared_count_of_rows_is_weighed_against_the_payload_before_a_row_is_held() {
        /// Bytes the fields of the box require: its own eight, then a row of four
        /// for every sample the count declares
        const NEEDED: u64 = 8 + u32::MAX as u64 * 4;

        let payload = b"\0\0\x01\0\xff\xff\xff\xff";

        assert!(matches!(
            TrackRunBox::decode_payload(payload),
            Err(DecodeError::Field(FieldReadError::UnexpectedEof {
                needed: NEEDED,
                available: 8
            }))
        ));
    }

    /// Payload of a run stating no per-sample field, so that its rows are empty
    fn empty_rows(sample_count: u32) -> Vec<u8> {
        let mut payload = vec![0; 4];
        payload.extend_from_slice(&sample_count.to_be_bytes());

        payload
    }

    #[test]
    fn a_run_of_empty_rows_is_read_up_to_the_rows_the_box_holds() {
        let sample_count = u32::try_from(MAXIMUM_EMPTY_ROWS).unwrap();

        let run = TrackRunBox::decode_payload(&empty_rows(sample_count)).unwrap();

        assert_eq!(
            run.samples().len(),
            usize::try_from(MAXIMUM_EMPTY_ROWS).unwrap()
        );
    }

    #[test]
    fn a_count_of_empty_rows_past_the_rows_the_box_holds_is_rejected() {
        let past_the_limit = u32::try_from(MAXIMUM_EMPTY_ROWS.saturating_add(1)).unwrap();

        assert!(matches!(
            TrackRunBox::decode_payload(&empty_rows(past_the_limit)),
            Err(DecodeError::UnsupportedEntryCount {
                declared,
                limit: MAXIMUM_EMPTY_ROWS
            }) if declared == u64::from(past_the_limit)
        ));
    }

    #[test]
    fn a_payload_holding_rows_past_the_count_it_declares_is_rejected() {
        let mut payload = encoded_payload(&track_run());
        payload.extend_from_slice(&[0; 8]);

        assert!(matches!(
            TrackRunBox::decode_payload(&payload),
            Err(DecodeError::Field(FieldReadError::TrailingBytes {
                remaining: 8
            }))
        ));
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let mut payload = encoded_payload(&track_run());
        *payload.first_mut().unwrap() = 2;

        assert!(matches!(
            TrackRunBox::decode_payload(&payload),
            Err(DecodeError::UnsupportedVersion(2))
        ));
    }
}
