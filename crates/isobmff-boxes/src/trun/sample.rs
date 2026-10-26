//! [`TrackRunSample`], one row of the table of a track fragment run

use isobmff_core::{Error, FieldReader, FieldWriter};

use crate::data_types::SampleFlags;
use crate::full_box::{read_composition_offset, write_composition_offset};

/// Flag mask of sample-duration-present
pub(super) const SAMPLE_DURATION_PRESENT: u32 = 0x00_0100;

/// Flag mask of sample-size-present
pub(super) const SAMPLE_SIZE_PRESENT: u32 = 0x00_0200;

/// Flag mask of sample-flags-present
pub(super) const SAMPLE_FLAGS_PRESENT: u32 = 0x00_0400;

/// Flag mask of sample-composition-time-offsets-present
pub(super) const SAMPLE_COMPOSITION_TIME_OFFSETS_PRESENT: u32 = 0x00_0800;

/// Flags of `tr_flags` that say which fields the rows carry
pub(super) const ROW_FLAGS: u32 = SAMPLE_DURATION_PRESENT
    | SAMPLE_SIZE_PRESENT
    | SAMPLE_FLAGS_PRESENT
    | SAMPLE_COMPOSITION_TIME_OFFSETS_PRESENT;

/// Sample of a track fragment run, one row of a [`TrackRunBox`](crate::TrackRunBox)
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
    pub(super) fn tr_flags(&self) -> u32 {
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
    pub(super) fn encoded_len(tr_flags: u32) -> u64 {
        u64::from((tr_flags & ROW_FLAGS).count_ones()).saturating_mul(4)
    }

    /// Reads a row carrying the fields `tr_flags` says are there, in a box of `version`
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends inside the row.
    pub(super) fn read(
        reader: &mut FieldReader<'_>,
        tr_flags: u32,
        version: u8,
    ) -> Result<Self, Error> {
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
    pub(super) fn write(&self, writer: &mut FieldWriter<'_>, version: u8) -> Result<(), Error> {
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
