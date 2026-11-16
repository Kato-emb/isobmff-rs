//! [`RateShareEntry`], how to share the available bitrate between the media of a track and the others, and its operation points

use alloc::vec::Vec;

use isobmff_core::{Error, FieldReader, FieldWriter};

use super::entry::{SampleGroupDescriptionEntry, from_class, of_grouping_type, read_whole};

use crate::table::{TableEntry, entries_len, read_entries, write_entries};

/// Operation point of a [`RateShareEntry`] that holds more than one
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct RateShareOperationPoint {
    /// Total bitrate available to share at this point, in kilobits per second
    pub available_bitrate: u32,
    /// Percentage of the available bitrate to allocate to the media, 0 for no information
    pub target_rate_share: u16,
}

impl TableEntry for RateShareOperationPoint {
    const ENCODED_LEN: u64 = 6;

    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            available_bitrate: reader.read_u32()?,
            target_rate_share: reader.read_u16()?,
        })
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(self.available_bitrate)?;
        writer.write_u16(self.target_rate_share)
    }
}

/// Target rate shares of a [`RateShareEntry`], for one operation point or more
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum RateShareOperationPoints {
    /// One operation point, whose share applies at every available bitrate
    Single {
        /// Percentage of the available bitrate to allocate to the media, 0 for no information
        target_rate_share: u16,
    },
    /// Operation points in increasing order of `available_bitrate`, more than
    /// one where the entry is written
    Multiple {
        /// The operation points
        operation_points: Vec<RateShareOperationPoint>,
    },
}

impl Default for RateShareOperationPoints {
    fn default() -> Self {
        Self::Single {
            target_rate_share: 0,
        }
    }
}

/// [`RateShareEntry`] (`rash`), ISO/IEC 14496-12 §10.2.2
///
/// How to share the available bitrate between the media of a track and the
/// others. `operation_point_count` is the number of operation points; a
/// [`Multiple`](RateShareOperationPoints::Multiple) of fewer than two is
/// refused when written, with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
/// `operation_point_count`, as is one whose `available_bitrate`s are not
/// positive and strictly increasing, for `available_bitrate`.
#[doc(alias = "rash")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct RateShareEntry {
    /// Target rate shares
    pub operation_points: RateShareOperationPoints,
    /// Bitrate above which to allocate no more to the media, in kilobits per
    /// second, 0 for no information
    pub maximum_bitrate: u32,
    /// Bitrate below which to allocate nothing to the media, in kilobits per
    /// second, 0 for no information
    pub minimum_bitrate: u32,
    /// Order in which tracks are discarded, the highest first
    pub discard_priority: u8,
}

impl TryFrom<&SampleGroupDescriptionEntry> for RateShareEntry {
    type Error = Error;

    fn try_from(entry: &SampleGroupDescriptionEntry) -> Result<Self, Error> {
        of_grouping_type(entry, b"rash")?;
        read_whole(&entry.data, |reader| {
            let operation_point_count = reader.read_u16()?;
            let operation_points = if operation_point_count == 1 {
                RateShareOperationPoints::Single {
                    target_rate_share: reader.read_u16()?,
                }
            } else {
                RateShareOperationPoints::Multiple {
                    operation_points: read_entries(
                        reader,
                        u32::from(operation_point_count),
                        RateShareOperationPoint::ENCODED_LEN,
                        RateShareOperationPoint::read,
                    )?,
                }
            };

            Ok(Self {
                operation_points,
                maximum_bitrate: reader.read_u32()?,
                minimum_bitrate: reader.read_u32()?,
                discard_priority: reader.read_bytes::<1>()?[0],
            })
        })
    }
}

impl TryFrom<&RateShareEntry> for SampleGroupDescriptionEntry {
    type Error = Error;

    fn try_from(class: &RateShareEntry) -> Result<Self, Error> {
        let operation_points_len = match &class.operation_points {
            RateShareOperationPoints::Single { .. } => 2,
            RateShareOperationPoints::Multiple { operation_points } => {
                entries_len(operation_points)
            }
        };
        from_class(
            b"rash",
            operation_points_len.saturating_add(2 + 4 + 4 + 1),
            |writer| {
                match &class.operation_points {
                    RateShareOperationPoints::Single { target_rate_share } => {
                        writer.write_u16(1)?;
                        writer.write_u16(*target_rate_share)?;
                    }
                    RateShareOperationPoints::Multiple { operation_points } => {
                        let operation_point_count = u16::try_from(operation_points.len())
                            .ok()
                            .filter(|count| *count > 1)
                            .ok_or_else(|| Error::invalid_value("operation_point_count"))?;
                        let increasing = operation_points
                            .iter()
                            .try_fold(0, |previous, point| {
                                (point.available_bitrate > previous)
                                    .then_some(point.available_bitrate)
                            })
                            .is_some();
                        if !increasing {
                            return Err(Error::invalid_value("available_bitrate"));
                        }
                        writer.write_u16(operation_point_count)?;
                        write_entries(writer, operation_points)?;
                    }
                }
                writer.write_u32(class.maximum_bitrate)?;
                writer.write_u32(class.minimum_bitrate)?;
                writer.write_bytes(&[class.discard_priority])
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::Error;

    use super::super::entry::{assert_round_trips, entry_of};
    use super::{RateShareEntry, RateShareOperationPoint, RateShareOperationPoints};

    #[test]
    fn reads_and_writes_the_bytes_its_syntax_lays_out() {
        assert_round_trips(&[
            (
                b"rash",
                b"\0\x01\0\x32\0\0\x01\0\0\0\0\x10\x80".as_slice(),
                RateShareEntry {
                    operation_points: RateShareOperationPoints::Single {
                        target_rate_share: 50,
                    },
                    maximum_bitrate: 256,
                    minimum_bitrate: 16,
                    discard_priority: 128,
                },
            ),
            (
                b"rash",
                b"\0\x02\0\0\0\x40\0\x14\0\0\x01\0\0\x1e\0\0\0\0\0\0\0\0\0".as_slice(),
                RateShareEntry {
                    operation_points: RateShareOperationPoints::Multiple {
                        operation_points: vec![
                            RateShareOperationPoint {
                                available_bitrate: 64,
                                target_rate_share: 20,
                            },
                            RateShareOperationPoint {
                                available_bitrate: 256,
                                target_rate_share: 30,
                            },
                        ],
                    },
                    ..RateShareEntry::default()
                },
            ),
        ]);
    }

    #[test]
    fn an_entry_of_another_grouping_type_is_refused() {
        assert_eq!(
            [
                RateShareEntry::try_from(&entry_of(
                    b"seig",
                    b"\0\x01\0\x32\0\0\x01\0\0\0\0\x10\x80"
                ))
                .map(|_| ()),
            ],
            [Err(Error::invalid_value("grouping_type")); 1]
        );
    }
}
