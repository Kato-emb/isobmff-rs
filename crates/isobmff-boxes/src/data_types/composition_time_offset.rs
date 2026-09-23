//! [`CompositionTimeOffset`], the composition time offset a `trun` row or a `ctts` entry carries, ISO/IEC 14496-12 §8.8.8, §8.6.1.3

use isobmff_core::{Error, FieldReader, FieldWidth, FieldWriter};

/// Widest composition time offset a `trun` row or a `ctts` entry carries, which version 0 writes unsigned
const COMPOSITION_TIME_OFFSET_MAXIMUM: i64 = u32::MAX as i64;

/// Lowest composition time offset a `trun` row or a `ctts` entry carries, which version 1 writes signed
const COMPOSITION_TIME_OFFSET_MINIMUM: i64 = i32::MIN as i64;

/// Composition time offset one of the two versions of a `trun` or a `ctts` writes
///
/// Version 0 of either box writes the offset unsigned in 32 bits and version 1
/// signed (ISO/IEC 14496-12 §8.8.8, §8.6.1.3), so a value in
/// `-2_147_483_648..=4_294_967_295` is one a row or an entry can carry, and
/// this holds such a value alone.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CompositionTimeOffset(i64);

impl CompositionTimeOffset {
    /// Creates the offset from its value
    ///
    /// Returns `None` when `offset` lies outside what either version writes.
    #[must_use]
    pub const fn new(offset: i64) -> Option<Self> {
        if offset < COMPOSITION_TIME_OFFSET_MINIMUM || offset > COMPOSITION_TIME_OFFSET_MAXIMUM {
            return None;
        }

        Some(Self(offset))
    }

    /// Returns the value of the offset
    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }

    /// Returns the version of a `trun` or a `ctts` that writes every one of `offsets`
    ///
    /// Version 1 when one of them is negative, and version 0 otherwise.
    /// Returns `None` when one is negative while another lies past
    /// [`i32::MAX`], which leaves no version able to write both.
    pub(crate) fn version_writing(offsets: impl Iterator<Item = Self> + Clone) -> Option<u8> {
        let signed = offsets.clone().any(|offset| offset.0.is_negative());
        let past_the_signed_range = offsets
            .into_iter()
            .any(|offset| offset.0 > i64::from(i32::MAX));

        // Why not version 1 throughout: §8.6.1.3 asks for the unsigned form
        // wherever it carries the offsets, which the readers of earlier brands
        // accept.
        match (signed, past_the_signed_range) {
            (true, true) => None,
            (true, false) => Some(1),
            (false, _) => Some(0),
        }
    }

    /// Reads the offset the way `version` writes it
    pub(crate) fn read(reader: &mut FieldReader<'_>, version: u8) -> Result<Self, Error> {
        Ok(Self(match version {
            0 => i64::from(reader.read_u32()?),
            _ => i64::from(reader.read_i32()?),
        }))
    }

    /// Writes the offset the way `version` writes it
    pub(crate) fn write(self, writer: &mut FieldWriter<'_>, version: u8) -> Result<(), Error> {
        // Why not unwrap: the constructors keep every offset within the version
        // `version_writing` picks, and where one slips through, the fallback
        // version 1 is refused here rather than written truncated.
        let out_of_range = Error::out_of_range(self.0.unsigned_abs(), FieldWidth::Compact);
        if version == 0 {
            writer.write_u32(u32::try_from(self.0).map_err(|_| out_of_range)?)
        } else {
            writer.write_i32(i32::try_from(self.0).map_err(|_| out_of_range)?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CompositionTimeOffset;

    #[test]
    fn an_offset_outside_what_either_version_writes_is_refused() {
        assert_eq!(CompositionTimeOffset::new(i64::from(u32::MAX) + 1), None);
        assert_eq!(CompositionTimeOffset::new(i64::from(i32::MIN) - 1), None);
    }
}
