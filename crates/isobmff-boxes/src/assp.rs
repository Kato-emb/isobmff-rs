//! [`AlternativeStartupSequencePropertiesBox`] and the offsets it bounds

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFlags,
    FullBoxHeader,
};

use crate::table::{TableEntry, counted_len, read_counted, write_counted};

/// Bound of the alternative startup sequences of one parameterized sample grouping, one entry of an [`AlternativeStartupSequencePropertiesBox`]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct AlternativeStartupSequenceOffsetEntry {
    /// `grouping_type_parameter` of the alternative startup sequence sample grouping the entry applies to
    pub grouping_type_parameter: u32,
    /// Least `sample_offset[1]` of the sample group description entries of that grouping
    pub min_initial_alt_startup_offset: i32,
}

impl TableEntry for AlternativeStartupSequenceOffsetEntry {
    const ENCODED_LEN: u64 = 4 + 4;

    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            grouping_type_parameter: reader.read_u32()?,
            min_initial_alt_startup_offset: reader.read_i32()?,
        })
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(self.grouping_type_parameter)?;
        writer.write_i32(self.min_initial_alt_startup_offset)
    }
}

/// Bounds an [`AlternativeStartupSequencePropertiesBox`] sets, in the layout its version selects
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum AlternativeStartupSequenceOffsets {
    /// Version 0: the bound of the grouping that version 0 of the `SampleToGroupBox` maps
    Unparameterized {
        /// Least `sample_offset[1]` of the sample group description entries of the grouping
        min_initial_alt_startup_offset: i32,
    },
    /// Version 1: the bounds of the groupings that version 1 of the `SampleToGroupBox` maps, one per `grouping_type_parameter`
    Parameterized {
        /// Bound of each grouping, in the order they came
        entries: Vec<AlternativeStartupSequenceOffsetEntry>,
    },
}

impl Default for AlternativeStartupSequenceOffsets {
    fn default() -> Self {
        Self::Unparameterized {
            min_initial_alt_startup_offset: 0,
        }
    }
}

/// [`AlternativeStartupSequencePropertiesBox`] (`assp`), ISO/IEC 14496-12 §8.8.16
///
/// The properties of the alternative startup sequence sample groups in the
/// track fragments after the movie, of the track its
/// `TrackExtensionPropertiesBox` names.
///
/// The version of the box follows the variant of
/// [`offsets`](Self::offsets), and is the version of the `SampleToGroupBox`
/// of the grouping.
#[doc(alias = "assp")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct AlternativeStartupSequencePropertiesBox {
    /// Bounds of the alternative startup sequence sample groupings
    pub offsets: AlternativeStartupSequenceOffsets,
}

impl BoxDefinition for AlternativeStartupSequencePropertiesBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"assp");
}

impl BoxDecode for AlternativeStartupSequencePropertiesBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let offsets = match FullBoxHeader::from_bytes(reader.read_bytes::<4>()?).version() {
            0 => AlternativeStartupSequenceOffsets::Unparameterized {
                min_initial_alt_startup_offset: reader.read_i32()?,
            },
            1 => AlternativeStartupSequenceOffsets::Parameterized {
                entries: read_counted(reader)?,
            },
            version => return Err(Error::unsupported_version(version)),
        };

        Ok(Self { offsets })
    }
}

impl BoxEncode for AlternativeStartupSequencePropertiesBox {
    fn payload_len(&self) -> u64 {
        match &self.offsets {
            AlternativeStartupSequenceOffsets::Unparameterized { .. } => 4 + 4,
            AlternativeStartupSequenceOffsets::Parameterized { entries } => {
                counted_len(entries).saturating_add(4)
            }
        }
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let version = u8::from(matches!(
            self.offsets,
            AlternativeStartupSequenceOffsets::Parameterized { .. }
        ));
        writer.write_bytes(&FullBoxHeader::new(version, FullBoxFlags::ZERO).to_bytes())?;
        match &self.offsets {
            AlternativeStartupSequenceOffsets::Unparameterized {
                min_initial_alt_startup_offset,
            } => writer.write_i32(*min_initial_alt_startup_offset),
            AlternativeStartupSequenceOffsets::Parameterized { entries } => {
                write_counted(writer, entries)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, Error};

    use super::{
        AlternativeStartupSequenceOffsetEntry, AlternativeStartupSequenceOffsets,
        AlternativeStartupSequencePropertiesBox,
    };
    use crate::encoded;

    /// `assp` of version 0 and a bound of -2
    const ASSP_V0: &[u8] = b"\0\0\0\x10assp\0\0\0\0\xff\xff\xff\xfe";

    /// `assp` of version 1: a bound of -2 for parameter 1 and of 3 for parameter 2
    const ASSP_V1: &[u8] = b"\0\0\0\x20assp\x01\0\0\0\0\0\0\x02\
        \0\0\0\x01\xff\xff\xff\xfe\0\0\0\x02\0\0\0\x03";

    fn unparameterized() -> AlternativeStartupSequencePropertiesBox {
        AlternativeStartupSequencePropertiesBox {
            offsets: AlternativeStartupSequenceOffsets::Unparameterized {
                min_initial_alt_startup_offset: -2,
            },
        }
    }

    fn parameterized() -> AlternativeStartupSequencePropertiesBox {
        AlternativeStartupSequencePropertiesBox {
            offsets: AlternativeStartupSequenceOffsets::Parameterized {
                entries: vec![
                    AlternativeStartupSequenceOffsetEntry {
                        grouping_type_parameter: 1,
                        min_initial_alt_startup_offset: -2,
                    },
                    AlternativeStartupSequenceOffsetEntry {
                        grouping_type_parameter: 2,
                        min_initial_alt_startup_offset: 3,
                    },
                ],
            },
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_in_either_version() {
        assert_eq!(
            (
                AlternativeStartupSequencePropertiesBox::decode(ASSP_V0),
                AlternativeStartupSequencePropertiesBox::decode(ASSP_V1)
            ),
            (
                Ok((unparameterized(), b"".as_slice())),
                Ok((parameterized(), b"".as_slice()))
            )
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_the_version_its_offsets_call_for() {
        assert_eq!(
            (encoded(&unparameterized()), encoded(&parameterized())),
            (ASSP_V0.to_vec(), ASSP_V1.to_vec())
        );
    }

    #[test]
    fn a_payload_ending_inside_the_bound_is_refused() {
        assert_eq!(
            AlternativeStartupSequencePropertiesBox::decode_payload(b"\0\0\0\0\xff\xff"),
            Err(Error::truncated_payload(8, 6))
        );
    }

    #[test]
    fn a_count_of_more_entries_than_the_payload_holds_is_refused() {
        assert_eq!(
            AlternativeStartupSequencePropertiesBox::decode_payload(
                b"\x01\0\0\0\0\0\0\x02\0\0\0\x01\xff\xff\xff\xfe"
            ),
            Err(Error::truncated_payload(24, 16))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            AlternativeStartupSequencePropertiesBox::decode_payload(b"\x02\0\0\0"),
            Err(Error::unsupported_version(2))
        );
    }
}
