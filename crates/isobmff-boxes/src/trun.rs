//! [`TrackRunBox`] (`trun`), ISO/IEC 14496-12 §8.8.8

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWidth, FieldWriter,
    FullBoxFields, FullBoxFlags,
};

use crate::data_types::{CompositionTimeOffset, SampleFlags, read_sample_flags};
use crate::tfhd::TrackFragmentHeaderBox;

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

/// One row of the table a track run documents, holding what it states per sample
///
/// Which fields a row carries is stated once for the whole run, so every row of
/// one [`TrackRunBox`] carries the same ones, and a field no row carries falls
/// back on the default the `tfhd` or the `trex` sets.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TrackRunSample {
    sample_duration: Option<u32>,
    sample_size: Option<u32>,
    sample_flags: Option<SampleFlags>,
    sample_composition_time_offset: Option<CompositionTimeOffset>,
}

impl TrackRunSample {
    /// Creates one row from the fields the run states for its sample
    #[must_use]
    pub const fn new(
        sample_duration: Option<u32>,
        sample_size: Option<u32>,
        sample_flags: Option<SampleFlags>,
        sample_composition_time_offset: Option<CompositionTimeOffset>,
    ) -> Self {
        Self {
            sample_duration,
            sample_size,
            sample_flags,
            sample_composition_time_offset,
        }
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
    pub const fn sample_flags(&self) -> Option<SampleFlags> {
        self.sample_flags
    }

    /// Returns the offset from the decode time of this sample to its composition time
    #[must_use]
    pub const fn sample_composition_time_offset(&self) -> Option<CompositionTimeOffset> {
        self.sample_composition_time_offset
    }
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
/// use isobmff_core::BoxEncode;
///
/// // Two samples, each stating its own size and nothing else
/// let samples = vec![
///     TrackRunSample::new(None, Some(1_024), None, None),
///     TrackRunSample::new(None, Some(2_048), None, None),
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
///             TrackRunSample::new(None, Some(1_024), None, None),
///             TrackRunSample::new(Some(512), Some(2_048), None, None),
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
    first_sample_flags: Option<SampleFlags>,
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
        first_sample_flags: Option<SampleFlags>,
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

        CompositionTimeOffset::version_writing(
            samples
                .iter()
                .filter_map(TrackRunSample::sample_composition_time_offset),
        )?;

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
    pub const fn first_sample_flags(&self) -> Option<SampleFlags> {
        self.first_sample_flags
    }

    /// Returns the samples of the run, in the order they are decoded
    #[must_use]
    pub fn samples(&self) -> &[TrackRunSample] {
        &self.samples
    }

    /// Returns the run as written against `tfhd`, every field it already defaults left out of the rows
    ///
    /// A field the header states a default for, which every row of the run
    /// agrees with, is left out of the rows. Flags that only the first row
    /// differs from the default on are written as its `first_sample_flags`
    /// (§8.8.8). Composition time offsets are left out of every row when none
    /// of them states one other than zero. Every sample keeps what it states.
    ///
    /// # Examples
    ///
    /// ```
    /// use isobmff_boxes::{CompositionTimeOffset, SampleFlags, TrackFragmentHeaderBox, TrackFragmentHeaderFlags, TrackRunBox, TrackRunSample};
    ///
    /// // Two samples lasting 1024 units each, of different sizes, every field stated
    /// let offset = Some(CompositionTimeOffset::new(0).unwrap());
    /// let track_run = TrackRunBox::new(
    ///     Some(100),
    ///     None,
    ///     vec![
    ///         TrackRunSample::new(Some(1_024), Some(4), Some(SampleFlags::ZERO), offset),
    ///         TrackRunSample::new(Some(1_024), Some(2), Some(SampleFlags::ZERO), offset),
    ///     ],
    /// )
    /// .unwrap();
    ///
    /// // Against a header stating the duration and the flags, only the size is written per row
    /// let header = TrackFragmentHeaderBox::new(TrackFragmentHeaderFlags::ZERO, 1, None, None, Some(1_024), None, Some(SampleFlags::ZERO));
    /// assert_eq!(
    ///     track_run.without_defaults(&header).samples(),
    ///     [
    ///         TrackRunSample::new(None, Some(4), None, None),
    ///         TrackRunSample::new(None, Some(2), None, None),
    ///     ]
    /// );
    /// ```
    #[must_use]
    pub fn without_defaults(&self, tfhd: &TrackFragmentHeaderBox) -> Self {
        let rows = || self.samples.iter();
        let carries_duration = tfhd.default_sample_duration().is_none_or(|default| {
            rows().any(|row| row.sample_duration.is_some_and(|value| value != default))
        });
        let carries_size = tfhd.default_sample_size().is_none_or(|default| {
            rows().any(|row| row.sample_size.is_some_and(|value| value != default))
        });
        let carries_offsets = rows().any(|row| {
            row.sample_composition_time_offset
                .is_some_and(|offset| offset.get() != 0)
        });
        let (carries_flags, first_sample_flags) = match tfhd.default_sample_flags() {
            Some(default)
                if rows()
                    .skip(1)
                    .all(|row| row.sample_flags.is_none_or(|flags| flags == default)) =>
            {
                let first = rows().next().and_then(|row| row.sample_flags);

                (
                    false,
                    self.first_sample_flags
                        .or(first)
                        .filter(|first| *first != default),
                )
            }
            _default_the_rows_do_not_share => (true, self.first_sample_flags),
        };

        let samples = rows()
            .map(|row| TrackRunSample {
                sample_duration: row.sample_duration.filter(|_| carries_duration),
                sample_size: row.sample_size.filter(|_| carries_size),
                sample_flags: row.sample_flags.filter(|_| carries_flags),
                sample_composition_time_offset: row
                    .sample_composition_time_offset
                    .filter(|_| carries_offsets),
            })
            .collect();

        Self {
            data_offset: self.data_offset,
            first_sample_flags,
            samples,
        }
    }
}

impl BoxDefinition for TrackRunBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"trun");
}

impl BoxDecode for TrackRunBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0 or 1.
    /// * [`UnsupportedFlags`](isobmff_core::ErrorKind::UnsupportedFlags): the box declares a
    ///   flag this box does not read, which stands for a field it cannot place, or
    ///   sample flags setting a bit §8.8.3.1 reserves.
    /// * [`ConflictingFlags`](isobmff_core::ErrorKind::ConflictingFlags): the box states the
    ///   flags of its first sample and of every sample at once.
    /// * [`UnsupportedEntryCount`](isobmff_core::ErrorKind::UnsupportedEntryCount): the rows
    ///   are empty and the `sample_count` is past the rows this box reads.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
    ///   ends inside a field the flags state, or holds fewer rows than the
    ///   `sample_count` declares. Bytes past those rows are the
    ///   [`TrailingPayload`](isobmff_core::ErrorKind::TrailingPayload) the payload
    ///   contract refuses.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let full_box = FullBoxFields::from_bytes(reader.read_bytes::<4>()?);
        let version = full_box.version();
        if version > 1 {
            return Err(Error::unsupported_version(version));
        }

        let flags = full_box.flags().bits();
        // Why not carrying an undefined bit through, as the `tfhd` does: §8.8.8
        // has the length of a row follow the bits set in the flags, so a bit this
        // box does not read stands for a field it cannot place in a row.
        let undefined = flags & !DEFINED_FLAGS;
        if undefined != 0 {
            return Err(Error::unsupported_flags(undefined));
        }
        let carries = |flag: u32| flags & flag != 0;
        if carries(FIRST_SAMPLE_FLAGS_PRESENT) && carries(SAMPLE_FLAGS_PRESENT) {
            return Err(Error::conflicting_flags(
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
            Some(read_sample_flags(reader)?)
        } else {
            None
        };

        let row_len =
            u64::from((flags & PER_SAMPLE_FLAGS).count_ones()).saturating_mul(OPTIONAL_FIELD_LEN);
        // Why not reading the rows and letting the reader report the shortfall:
        // the count comes from the input, so a row length of four bytes lets a
        // twelve-byte payload declare four billion rows, and the reading would
        // hold a gigabyte of them before the payload ran out.
        reader.require(row_len.saturating_mul(u64::from(sample_count)))?;

        let declared = u64::from(sample_count);
        if row_len == 0 && declared > MAXIMUM_EMPTY_ROWS {
            return Err(Error::unsupported_entry_count(declared, MAXIMUM_EMPTY_ROWS));
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
                Some(read_sample_flags(reader)?)
            } else {
                None
            };
            let sample_composition_time_offset = if carries(SAMPLE_COMPOSITION_TIME_OFFSETS_PRESENT)
            {
                Some(CompositionTimeOffset::read(reader, version)?)
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
        let rows = row.saturating_mul(self.samples.len() as u64);

        length.saturating_add(rows)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let bits = per_sample_field_flags(&self.samples)
            | self.data_offset.map_or(0, |_| DATA_OFFSET_PRESENT)
            | self
                .first_sample_flags
                .map_or(0, |_| FIRST_SAMPLE_FLAGS_PRESENT);

        // Why not unwrap: `TrackRunBox::new` refuses rows no one version writes,
        // and were one to slip through, version 1 refuses the offset past its
        // range when it is written.
        let version = CompositionTimeOffset::version_writing(
            self.samples
                .iter()
                .filter_map(TrackRunSample::sample_composition_time_offset),
        )
        .unwrap_or(1);

        // Why not unwrap: the bits are the flags this box defines, which lie
        // inside the field by construction, so the failure named here is one the
        // call cannot reach.
        let flags = FullBoxFlags::new(bits)
            .ok_or_else(|| Error::out_of_range(u64::from(bits), FieldWidth::Compact))?;

        writer.write_bytes(&FullBoxFields::new(version, flags).to_bytes())?;
        let sample_count = self.samples.len() as u64;
        // Why not saturate silently: a row count past `u32` cannot be written at
        // all, and the box has already declared a length built from it, so this
        // stands for a `Vec` no target can hold.
        writer.write_unsigned(FieldWidth::Compact, sample_count)?;
        if let Some(data_offset) = self.data_offset {
            writer.write_i32(data_offset)?;
        }
        if let Some(first_sample_flags) = self.first_sample_flags {
            writer.write_u32(first_sample_flags.bits())?;
        }

        for sample in &self.samples {
            for field in [
                sample.sample_duration,
                sample.sample_size,
                sample.sample_flags.map(SampleFlags::bits),
            ]
            .into_iter()
            .flatten()
            {
                writer.write_u32(field)?;
            }
            if let Some(offset) = sample.sample_composition_time_offset {
                offset.write(writer, version)?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests;
