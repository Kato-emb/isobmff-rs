//! [`SampleGroupDescriptionEntry`], an entry of a `SampleGroupDescriptionBox` as its bytes, and the conversion between it and the class derived for its `grouping_type`

use alloc::vec;
use alloc::vec::Vec;

use isobmff_core::{Error, FieldReader, FieldWriter, FourCc};

/// An entry of a `SampleGroupDescriptionBox`, as its bytes
///
/// [`SampleGroupDescriptionEntry`], ISO/IEC 14496-12 §8.9.3
///
/// The syntax of an entry is the class derived for the `grouping_type` of
/// the box it belongs to, so the box keeps each entry as its bytes, and a
/// caller who wants its fields reads it into that class by an explicit
/// conversion, and writes it back the same way. The classes ISO/IEC 14496-12
/// defines, by `grouping_type`:
///
/// * `roll`: [`RollRecoveryEntry`](crate::RollRecoveryEntry); `prol`:
///   [`AudioPreRollEntry`](crate::AudioPreRollEntry).
/// * `rash`: [`RateShareEntry`](crate::RateShareEntry).
/// * `alst`: [`AlternativeStartupEntry`](crate::AlternativeStartupEntry).
/// * `rap `: [`VisualRandomAccessEntry`](crate::VisualRandomAccessEntry).
/// * `tele`: [`TemporalLevelEntry`](crate::TemporalLevelEntry).
/// * `sap `: [`SapEntry`](crate::SapEntry).
/// * `stmi`: [`SampleToMetadataItemEntry`](crate::SampleToMetadataItemEntry).
/// * `drap`: [`VisualDrapEntry`](crate::VisualDrapEntry); `edrp`:
///   [`VisualEdrapEntry`](crate::VisualEdrapEntry).
/// * `pasr`: [`PixelAspectRatioEntry`](crate::PixelAspectRatioEntry).
/// * `casg`: [`CleanApertureEntry`](crate::CleanApertureEntry).
/// * `esgh`:
///   [`EssentialDescriptionsHierarchyEntry`](crate::EssentialDescriptionsHierarchyEntry).
/// * `it35`: [`T35SampleGroupEntry`](crate::T35SampleGroupEntry).
///
/// Reading an entry into a class refuses one of another `grouping_type` with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
/// `grouping_type`, one whose bytes end inside its fields with
/// [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload), and one
/// whose bytes run past its fields with
/// [`BufferLengthMismatch`](isobmff_core::ErrorKind::BufferLengthMismatch).
/// Writing a class into an entry refuses a value the specification does not
/// allow, as the class states.
///
/// ```
/// use isobmff_boxes::{RollRecoveryEntry, SampleGroupDescriptionEntry};
/// use isobmff_core::FourCc;
///
/// // An entry of a `roll` sample group description, of distance -2
/// let mut entry = SampleGroupDescriptionEntry::default();
/// entry.grouping_type = FourCc::new(*b"roll");
/// entry.data = vec![0xff, 0xfe];
///
/// // Read it into its class, and write it back
/// let roll = RollRecoveryEntry::try_from(&entry)?;
/// assert_eq!(roll.roll_distance, -2);
/// assert_eq!(SampleGroupDescriptionEntry::try_from(&roll)?, entry);
/// # Ok::<(), isobmff_core::Error>(())
/// ```
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SampleGroupDescriptionEntry {
    /// The `grouping_type` of the box the entry belongs to, which decides its syntax
    pub grouping_type: FourCc,
    /// The bytes of the entry, which the class derived for the `grouping_type` lays out
    pub data: Vec<u8>,
}

impl Default for SampleGroupDescriptionEntry {
    fn default() -> Self {
        Self {
            grouping_type: FourCc::new([0; 4]),
            data: Vec::new(),
        }
    }
}

/// A class derived from [`SampleGroupDescriptionEntry`] for one `grouping_type`
pub(super) trait DerivedEntry: Sized {
    /// The `grouping_type` whose entries the class lays out
    const GROUPING_TYPE: FourCc;

    /// Reads the fields of the class off the front of `reader`
    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error>;

    /// Returns how many bytes the fields take, saturating at `u64::MAX`
    fn encoded_len(&self) -> u64;

    /// Writes the fields onto the front of what `writer` has left, refusing a value the specification does not allow
    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error>;
}

/// Reads `entry` into the class `Class`, as [`SampleGroupDescriptionEntry`] sets out
pub(super) fn read_class<Class: DerivedEntry>(
    entry: &SampleGroupDescriptionEntry,
) -> Result<Class, Error> {
    if entry.grouping_type != Class::GROUPING_TYPE {
        return Err(Error::invalid_value("grouping_type"));
    }
    let mut reader = FieldReader::new(&entry.data);
    let class = Class::read(&mut reader)?;
    let left = reader.remainder().len();
    if left != 0 {
        let length = |length: usize| u64::try_from(length).unwrap_or(u64::MAX);
        return Err(Error::buffer_length_mismatch(
            length(entry.data.len().saturating_sub(left)),
            length(entry.data.len()),
        ));
    }

    Ok(class)
}

/// Writes `class` into an entry of its `grouping_type`, as [`SampleGroupDescriptionEntry`] sets out
pub(super) fn write_class<Class: DerivedEntry>(
    class: &Class,
) -> Result<SampleGroupDescriptionEntry, Error> {
    let mut data = vec![0; usize::try_from(class.encoded_len()).unwrap_or(usize::MAX)];
    let mut writer = FieldWriter::new(&mut data);
    class.write(&mut writer)?;
    writer.finish()?;

    Ok(SampleGroupDescriptionEntry {
        grouping_type: Class::GROUPING_TYPE,
        data,
    })
}

/// An entry of `grouping_type` of the bytes `data`
#[cfg(test)]
pub(super) fn entry_of(grouping_type: &[u8; 4], data: &[u8]) -> SampleGroupDescriptionEntry {
    SampleGroupDescriptionEntry {
        grouping_type: FourCc::new(*grouping_type),
        data: data.to_vec(),
    }
}

/// Asserts that each of `cases`, a `grouping_type`, the bytes its syntax lays out and the value of the class they hold, reads into its value and writes back into its bytes
#[cfg(test)]
pub(super) fn assert_round_trips<Class>(cases: &[(&[u8; 4], &[u8], Class)])
where
    Class: for<'entry> TryFrom<&'entry SampleGroupDescriptionEntry, Error = Error>
        + Clone
        + PartialEq
        + core::fmt::Debug,
    SampleGroupDescriptionEntry: for<'class> TryFrom<&'class Class, Error = Error>,
{
    assert_eq!(
        cases
            .iter()
            .map(|(grouping_type, bytes, _)| Class::try_from(&entry_of(grouping_type, bytes)))
            .collect::<Vec<_>>(),
        cases
            .iter()
            .map(|(_, _, value)| Ok(value.clone()))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        cases
            .iter()
            .map(|(_, _, value)| SampleGroupDescriptionEntry::try_from(value))
            .collect::<Vec<_>>(),
        cases
            .iter()
            .map(|(grouping_type, bytes, _)| Ok(entry_of(grouping_type, bytes)))
            .collect::<Vec<_>>()
    );
}

#[cfg(test)]
mod tests {
    use isobmff_core::Error;

    use super::entry_of;
    use crate::PixelAspectRatioEntry;

    #[test]
    fn an_entry_whose_bytes_end_inside_or_run_past_its_fields_is_refused() {
        assert_eq!(
            [
                PixelAspectRatioEntry::try_from(&entry_of(b"pasr", b"\0\0\0\x04\0\0\0")),
                PixelAspectRatioEntry::try_from(&entry_of(b"pasr", b"\0\0\0\x04\0\0\0\x03\x09")),
            ],
            [
                Err(Error::truncated_payload(8, 7)),
                Err(Error::buffer_length_mismatch(8, 9))
            ]
        );
    }
}
