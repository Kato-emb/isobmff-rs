//! [`HeaderDuration`], the `duration` of a movie, track or media header, ISO/IEC 14496-12 §8.2.2.3, §8.3.2.3, §8.4.2.3

use isobmff_core::{Error, FieldReader, FieldWidth, FieldWriter};

/// All 1s of a field of 32 bits
const ALL_ONES_COMPACT: u64 = u32::MAX as u64;

/// Duration an `mvhd`, a `tkhd` or an `mdhd` states, in the time scale its box counts in
///
/// ISO/IEC 14496-12 §8.2.2.3, §8.3.2.3 and §8.4.2.3 have each of the three
/// headers set its `duration` to all 1s when the duration cannot be
/// determined, so a duration is either a value below [`u64::MAX`] or
/// [`INDETERMINATE`](Self::INDETERMINATE).
///
/// A header written at version 0 stating `0xFFFF_FFFF` reads as
/// [`INDETERMINATE`](Self::INDETERMINATE); a duration of 2^32 − 1 is written at
/// version 1, so a header holding it reads back.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::HeaderDuration;
///
/// // A duration of five seconds on a millisecond time scale
/// assert_eq!(HeaderDuration::new(5_000).map(HeaderDuration::get), Some(Some(5_000)));
///
/// // All 1s is the duration that cannot be determined, not a value
/// assert_eq!(HeaderDuration::new(u64::MAX), None);
/// assert_eq!(HeaderDuration::INDETERMINATE.get(), None);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct HeaderDuration(u64);

impl HeaderDuration {
    /// Duration that cannot be determined, which a header writes as all 1s
    pub const INDETERMINATE: Self = Self(u64::MAX);

    /// Duration of 0
    pub const ZERO: Self = Self(0);

    /// Creates the duration from its value
    ///
    /// Returns `None` when `duration` is [`u64::MAX`], the all 1s that states a
    /// duration cannot be determined.
    #[must_use]
    pub const fn new(duration: u64) -> Option<Self> {
        if duration == u64::MAX {
            return None;
        }

        Some(Self(duration))
    }

    /// Creates the duration from one derived from other boxes
    ///
    /// A `duration` of `None`, or of [`u64::MAX`], cannot be determined.
    pub(crate) fn from_derived(duration: Option<u64>) -> Self {
        duration.and_then(Self::new).unwrap_or(Self::INDETERMINATE)
    }

    /// Returns the value of the duration, or `None` where it cannot be determined
    #[must_use]
    pub const fn get(self) -> Option<u64> {
        if self.0 == u64::MAX {
            None
        } else {
            Some(self.0)
        }
    }

    /// Returns whether the duration can be written in 32 bits without reading back as one that cannot be determined
    pub(crate) const fn fits_in_32_bits(self) -> bool {
        self.0 == u64::MAX || self.0 < ALL_ONES_COMPACT
    }

    /// Reads the duration in the width its header settled
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
    ///   ends inside the field.
    pub(crate) fn read(reader: &mut FieldReader<'_>, width: FieldWidth) -> Result<Self, Error> {
        let duration = reader.read_unsigned(width)?;
        let compact_all_ones = width == FieldWidth::Compact && duration == ALL_ONES_COMPACT;

        Ok(if compact_all_ones {
            Self::INDETERMINATE
        } else {
            Self(duration)
        })
    }

    /// Writes the duration in the width its header settled
    ///
    /// # Errors
    ///
    /// * The failures of [`FieldWriter::write_unsigned`].
    pub(crate) fn write(
        self,
        writer: &mut FieldWriter<'_>,
        width: FieldWidth,
    ) -> Result<(), Error> {
        let compact_indeterminate = width == FieldWidth::Compact && self == Self::INDETERMINATE;

        writer.write_unsigned(
            width,
            if compact_indeterminate {
                ALL_ONES_COMPACT
            } else {
                self.0
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{FieldReader, FieldWidth, FieldWriter};

    use super::HeaderDuration;

    #[test]
    fn a_duration_that_cannot_be_determined_is_all_1s_at_either_width() {
        let mut buffer = [0; 12];
        let mut writer = FieldWriter::new(&mut buffer);
        HeaderDuration::INDETERMINATE
            .write(&mut writer, FieldWidth::Compact)
            .unwrap();
        HeaderDuration::INDETERMINATE
            .write(&mut writer, FieldWidth::Extended)
            .unwrap();
        writer.finish().unwrap();

        assert_eq!(buffer, [0xff; 12]);

        let mut reader = FieldReader::new(&buffer);
        assert_eq!(
            HeaderDuration::read(&mut reader, FieldWidth::Compact),
            Ok(HeaderDuration::INDETERMINATE)
        );
        assert_eq!(
            HeaderDuration::read(&mut reader, FieldWidth::Extended),
            Ok(HeaderDuration::INDETERMINATE)
        );
    }
}
