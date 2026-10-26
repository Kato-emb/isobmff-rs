//! [`SubsegmentIndexBox`] and the [`Subsegment`]s it divides into levels

use alloc::vec::Vec;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::full_box::{read_version_zero, write_version_zero};
use crate::table::{
    TableEntry, counted_len, read_counted, read_entries, write_counted, write_entry_count,
};

/// Byte range of a partial subsegment and the level it is assigned to, one range of a [`Subsegment`]
///
/// `range_size` is a 24-bit value. A value wider than that is refused when
/// the range is written, with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for `range_size`.
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct SubsegmentRange {
    /// Level the partial subsegment is assigned to
    pub level: u8,
    /// Size of the partial subsegment, in bytes; 0 in the last range of a
    /// subsegment stands for the rest of the segment
    pub range_size: u32,
}

impl TableEntry for SubsegmentRange {
    const ENCODED_LEN: u64 = 4;

    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let packed = reader.read_u32()?;
        let [level, ..] = packed.to_be_bytes();

        Ok(Self {
            level,
            range_size: packed & 0x00ff_ffff,
        })
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.range_size > 0x00ff_ffff {
            return Err(Error::invalid_value("range_size"));
        }

        writer.write_u32(u32::from(self.level) << 24 | self.range_size)
    }
}

/// Partial subsegments of one subsegment, one entry of a [`SubsegmentIndexBox`]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct Subsegment {
    /// Partial subsegments the media data of the subsegment is grouped into, in increasing level
    pub ranges: Vec<SubsegmentRange>,
}

/// [`SubsegmentIndexBox`] (`ssix`), ISO/IEC 14496-12 §8.14.4
///
/// For each subsegment the `SegmentIndexBox` right before it indexes, the
/// byte ranges of its partial subsegments and the level, as the
/// `LevelAssignmentBox` specifies levels, each is assigned to.
///
/// No subsegment, or a subsegment of fewer than two ranges, which the
/// specification does not allow, reads, but writing it is refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
/// `subsegment_count` or `range_count`.
#[doc(alias = "ssix")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct SubsegmentIndexBox {
    /// Subsegments, as many as the references of the `SegmentIndexBox` right before the box
    pub subsegments: Vec<Subsegment>,
}

impl BoxDefinition for SubsegmentIndexBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"ssix");
}

impl BoxDecode for SubsegmentIndexBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;
        let subsegment_count = reader.read_u32()?;
        let subsegments = read_entries(reader, subsegment_count, 4, |reader| {
            Ok(Subsegment {
                ranges: read_counted(reader)?,
            })
        })?;

        Ok(Self { subsegments })
    }
}

impl BoxEncode for SubsegmentIndexBox {
    fn payload_len(&self) -> u64 {
        self.subsegments
            .iter()
            .map(|subsegment| counted_len(&subsegment.ranges))
            .fold(4 + 4, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.subsegments.is_empty() {
            return Err(Error::invalid_value("subsegment_count"));
        }
        if self
            .subsegments
            .iter()
            .any(|subsegment| subsegment.ranges.len() < 2)
        {
            return Err(Error::invalid_value("range_count"));
        }

        write_version_zero(writer)?;
        write_entry_count(writer, self.subsegments.len())?;
        self.subsegments
            .iter()
            .try_for_each(|subsegment| write_counted(writer, &subsegment.ranges))
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::{Subsegment, SubsegmentIndexBox, SubsegmentRange};
    use crate::encoded;

    /// `ssix` of two subsegments: levels 1 and 2 of 0x100 and 0x200 bytes, then levels 1 and 2 of 0x300 bytes and the rest
    const SSIX: &[u8] = b"\0\0\0\x28ssix\0\0\0\0\0\0\0\x02\
        \0\0\0\x02\x01\0\x01\0\x02\0\x02\0\
        \0\0\0\x02\x01\0\x03\0\x02\0\0\0";

    fn range(level: u8, range_size: u32) -> SubsegmentRange {
        SubsegmentRange { level, range_size }
    }

    fn subsegment_index() -> SubsegmentIndexBox {
        SubsegmentIndexBox {
            subsegments: vec![
                Subsegment {
                    ranges: vec![range(1, 0x100), range(2, 0x200)],
                },
                Subsegment {
                    ranges: vec![range(1, 0x300), range(2, 0)],
                },
            ],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            SubsegmentIndexBox::decode(SSIX),
            Ok((subsegment_index(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&subsegment_index()), SSIX);
    }

    #[test]
    fn values_the_specification_does_not_allow_are_refused_when_written() {
        let mut one_range = subsegment_index();
        one_range.subsegments = vec![Subsegment {
            ranges: vec![range(1, 0)],
        }];
        let mut wide_range = subsegment_index();
        wide_range.subsegments = vec![Subsegment {
            ranges: vec![range(1, 0x0100_0000), range(2, 0)],
        }];

        assert_eq!(
            (
                SubsegmentIndexBox::default().encode(&mut [0; 64]),
                one_range.encode(&mut [0; 64]),
                wide_range.encode(&mut [0; 64])
            ),
            (
                Err(Error::invalid_value("subsegment_count")),
                Err(Error::invalid_value("range_count")),
                Err(Error::invalid_value("range_size"))
            )
        );
    }

    #[test]
    fn a_count_of_more_ranges_than_the_payload_holds_is_refused() {
        assert_eq!(
            SubsegmentIndexBox::decode_payload(b"\0\0\0\0\0\0\0\x01\0\0\0\x03\x01\0\x01\0"),
            Err(Error::truncated_payload(12 + 3 * 4, 16))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            SubsegmentIndexBox::decode_payload(b"\x01\0\0\0\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
