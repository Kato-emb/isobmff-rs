//! [`LevelAssignmentBox`] and the [`Level`]s it assigns

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FourCc,
};

use crate::full_box::{read_version_zero, write_version_zero};
use crate::table::read_entries;

/// Mechanism that assigns the samples of a track to a [`Level`], selected by its `assignment_type`
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LevelAssignment {
    /// `assignment_type` 0: the samples of the track mapped to entry `n` of
    /// the sample group description of `grouping_type` lie in level `n`
    SampleGroup {
        /// Sample grouping that maps the samples to levels
        grouping_type: FourCc,
    },
    /// `assignment_type` 1: as [`SampleGroup`](Self::SampleGroup), by a
    /// parameterized sample grouping
    ParameterizedSampleGroup {
        /// Sample grouping that maps the samples to levels
        grouping_type: FourCc,
        /// Parameter of the sample grouping
        grouping_type_parameter: u32,
    },
    /// `assignment_type` 2 or 3: the level holds the track
    ///
    /// The two differ in how a `SubsegmentIndexBox` is processed: under 3,
    /// the last `MediaDataBox` of a partial subsegment is complete. Any other
    /// `assignment_type` is refused when the box is written, with
    /// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
    /// `assignment_type`.
    Track {
        /// `assignment_type`, 2 or 3
        assignment_type: u8,
    },
    /// `assignment_type` 4: the level holds the samples of a sub-track
    SubTrack {
        /// `sub_track_ID` of the sub-track, as its `SubTrackBox` gives it
        sub_track_id: u32,
    },
}

/// Level of a fraction, one entry of a [`LevelAssignmentBox`]
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Level {
    /// `track_ID` of the track assigned to the level
    pub track_id: u32,
    /// Whether a fraction can be formed by concatenating any number of levels
    /// and padding the last `MediaDataBox` with zero bytes up to its size
    ///
    /// The use of the flag is deprecated.
    pub padding_flag: bool,
    /// How the samples of the track are assigned to the level
    pub assignment: LevelAssignment,
}

/// [`LevelAssignmentBox`] (`leva`), ISO/IEC 14496-12 §8.8.13
///
/// Maps features such as scalability layers to levels, the subsets of the
/// movie fragments after the initial movie whose data appears contiguously,
/// in increasing order of level, within each fraction.
///
/// An `assignment_type` the specification reserves gives no length to the
/// level, so a box holding one does not read: it is refused with
/// [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
/// `assignment_type`, and a `MovieExtendsBox` keeps it as its bytes. Fewer
/// than two levels, which the specification does not allow, read, but
/// writing them is refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
/// `level_count`, as are more than 8 bits count.
#[doc(alias = "leva")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct LevelAssignmentBox {
    /// Levels each fraction is grouped into, level `j` the `j`th of them from 1
    pub levels: Vec<Level>,
}

impl BoxDefinition for LevelAssignmentBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"leva");
}

impl BoxDecode for LevelAssignmentBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;
        let [level_count] = *reader.read_bytes::<1>()?;
        let levels = read_entries(reader, u32::from(level_count), 4 + 1, |reader| {
            let track_id = reader.read_u32()?;
            let [packed] = *reader.read_bytes::<1>()?;
            let assignment = match packed & 0x7f {
                0 => LevelAssignment::SampleGroup {
                    grouping_type: FourCc::new(*reader.read_bytes::<4>()?),
                },
                1 => LevelAssignment::ParameterizedSampleGroup {
                    grouping_type: FourCc::new(*reader.read_bytes::<4>()?),
                    grouping_type_parameter: reader.read_u32()?,
                },
                assignment_type @ (2 | 3) => LevelAssignment::Track { assignment_type },
                4 => LevelAssignment::SubTrack {
                    sub_track_id: reader.read_u32()?,
                },
                _ => return Err(Error::unsupported_value("assignment_type")),
            };

            Ok(Level {
                track_id,
                padding_flag: packed & 0x80 != 0,
                assignment,
            })
        })?;

        Ok(Self { levels })
    }
}

impl BoxEncode for LevelAssignmentBox {
    fn payload_len(&self) -> u64 {
        self.levels
            .iter()
            .map(|level| match level.assignment {
                LevelAssignment::SampleGroup { .. } | LevelAssignment::SubTrack { .. } => 4 + 1 + 4,
                LevelAssignment::ParameterizedSampleGroup { .. } => 4 + 1 + 4 + 4,
                LevelAssignment::Track { .. } => 4 + 1,
            })
            .fold(4 + 1, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let level_count =
            u8::try_from(self.levels.len()).map_err(|_| Error::invalid_value("level_count"))?;
        if level_count < 2 {
            return Err(Error::invalid_value("level_count"));
        }

        write_version_zero(writer)?;
        writer.write_bytes(&[level_count])?;
        for level in &self.levels {
            writer.write_u32(level.track_id)?;
            let padding_flag = u8::from(level.padding_flag) << 7;
            match level.assignment {
                LevelAssignment::SampleGroup { grouping_type } => {
                    writer.write_bytes(&[padding_flag])?;
                    writer.write_bytes(grouping_type.as_bytes())?;
                }
                LevelAssignment::ParameterizedSampleGroup {
                    grouping_type,
                    grouping_type_parameter,
                } => {
                    writer.write_bytes(&[padding_flag | 1])?;
                    writer.write_bytes(grouping_type.as_bytes())?;
                    writer.write_u32(grouping_type_parameter)?;
                }
                LevelAssignment::Track {
                    assignment_type: assignment_type @ (2 | 3),
                } => writer.write_bytes(&[padding_flag | assignment_type])?,
                LevelAssignment::Track { .. } => {
                    return Err(Error::invalid_value("assignment_type"));
                }
                LevelAssignment::SubTrack { sub_track_id } => {
                    writer.write_bytes(&[padding_flag | 4])?;
                    writer.write_u32(sub_track_id)?;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error, FourCc};

    use super::{Level, LevelAssignment, LevelAssignmentBox};
    use crate::encoded;

    /// `leva` of four levels: track 1 by track (type 2), track 2 by `tele` with padding, track 2 by `tele` parameter 7, and sub-track 3 of track 2
    const LEVA: &[u8] = b"\0\0\0\x31leva\0\0\0\0\x04\
        \0\0\0\x01\x02\
        \0\0\0\x02\x80tele\
        \0\0\0\x02\x01tele\0\0\0\x07\
        \0\0\0\x02\x04\0\0\0\x03";

    fn level_assignment() -> LevelAssignmentBox {
        LevelAssignmentBox {
            levels: vec![
                Level {
                    track_id: 1,
                    padding_flag: false,
                    assignment: LevelAssignment::Track { assignment_type: 2 },
                },
                Level {
                    track_id: 2,
                    padding_flag: true,
                    assignment: LevelAssignment::SampleGroup {
                        grouping_type: FourCc::new(*b"tele"),
                    },
                },
                Level {
                    track_id: 2,
                    padding_flag: false,
                    assignment: LevelAssignment::ParameterizedSampleGroup {
                        grouping_type: FourCc::new(*b"tele"),
                        grouping_type_parameter: 7,
                    },
                },
                Level {
                    track_id: 2,
                    padding_flag: false,
                    assignment: LevelAssignment::SubTrack { sub_track_id: 3 },
                },
            ],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            LevelAssignmentBox::decode(LEVA),
            Ok((level_assignment(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&level_assignment()), LEVA);
    }

    #[test]
    fn an_assignment_type_the_specification_reserves_is_refused() {
        assert_eq!(
            LevelAssignmentBox::decode_payload(b"\0\0\0\0\x02\0\0\0\x01\x05\0\0\0\x01\x02"),
            Err(Error::unsupported_value("assignment_type"))
        );
    }

    #[test]
    fn values_the_specification_does_not_allow_are_refused_when_written() {
        let mut one_level = level_assignment();
        one_level.levels.truncate(1);
        let mut by_track_of_type_4 = level_assignment();
        by_track_of_type_4.levels.first_mut().unwrap().assignment =
            LevelAssignment::Track { assignment_type: 4 };

        assert_eq!(
            (
                one_level.encode(&mut [0; 64]),
                by_track_of_type_4.encode(&mut [0; 64])
            ),
            (
                Err(Error::invalid_value("level_count")),
                Err(Error::invalid_value("assignment_type"))
            )
        );
    }

    #[test]
    fn a_payload_ending_inside_a_level_is_refused() {
        assert_eq!(
            LevelAssignmentBox::decode_payload(LEVA.get(8..35).unwrap()),
            Err(Error::truncated_payload(28, 27))
        );
    }

    #[test]
    fn a_count_of_more_levels_than_the_payload_holds_is_refused() {
        assert_eq!(
            LevelAssignmentBox::decode_payload(b"\0\0\0\0\x03\0\0\0\x01\x02"),
            Err(Error::truncated_payload(20, 10))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            LevelAssignmentBox::decode_payload(b"\x01\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
