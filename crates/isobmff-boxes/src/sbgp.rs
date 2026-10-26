//! [`SampleToGroupBox`] and the [`SampleToGroupEntry`]s it holds

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FourCc,
    FullBoxFlags, FullBoxHeader,
};

use crate::table::{TableEntry, counted_len, read_counted, write_counted};

/// Run of consecutive samples of one sample group, one entry of a [`SampleToGroupBox`]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct SampleToGroupEntry {
    /// Number of consecutive samples that the same sample group description entry describes
    pub sample_count: u32,
    /// Index, from 1, of the sample group description entry that describes
    /// these samples, or 0 where they are a member of no group of this type
    ///
    /// In a track fragment, an index of `0x10000` plus `n` names the entry `n`
    /// of the `SampleGroupDescriptionBox` of the same track fragment.
    pub group_description_index: u32,
}

impl TableEntry for SampleToGroupEntry {
    const ENCODED_LEN: u64 = 8;

    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            sample_count: reader.read_u32()?,
            group_description_index: reader.read_u32()?,
        })
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(self.sample_count)?;
        writer.write_u32(self.group_description_index)
    }
}

/// [`SampleToGroupBox`] (`sbgp`), ISO/IEC 14496-12 §8.9.2
///
/// The sample group of each sample of a track or track fragment, of one
/// sample grouping, as runs of samples of one group. Samples past the last
/// run are of the group the `default_group_description_index` of the
/// `SampleGroupDescriptionBox` of the same `grouping_type` names, if any, or
/// else of no group.
///
/// The box is of version 1 where it holds a `grouping_type_parameter`, and of
/// version 0 where it does not.
///
/// [`Default`] is the box of `grouping_type` four zero bytes, of no
/// parameter and no run.
#[doc(alias = "sbgp")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SampleToGroupBox {
    /// Criterion the samples are grouped by, which links the box to the
    /// `SampleGroupDescriptionBox` of the same `grouping_type`
    pub grouping_type: FourCc,
    /// Sub-type of the grouping, whose meaning the `grouping_type` defines
    pub grouping_type_parameter: Option<u32>,
    /// Runs of samples, in decoding order
    pub entries: Vec<SampleToGroupEntry>,
}

impl Default for SampleToGroupBox {
    fn default() -> Self {
        Self {
            grouping_type: FourCc::new([0; 4]),
            grouping_type_parameter: None,
            entries: Vec::new(),
        }
    }
}

impl BoxDefinition for SampleToGroupBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"sbgp");
}

impl BoxDecode for SampleToGroupBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?).version();
        if version > 1 {
            return Err(Error::unsupported_version(version));
        }
        let grouping_type = FourCc::new(*reader.read_bytes::<4>()?);
        let grouping_type_parameter = if version == 1 {
            Some(reader.read_u32()?)
        } else {
            None
        };

        Ok(Self {
            grouping_type,
            grouping_type_parameter,
            entries: read_counted(reader)?,
        })
    }
}

impl BoxEncode for SampleToGroupBox {
    fn payload_len(&self) -> u64 {
        let fields_len = if self.grouping_type_parameter.is_some() {
            4 + 4 + 4
        } else {
            4 + 4
        };

        counted_len(&self.entries).saturating_add(fields_len)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let version = u8::from(self.grouping_type_parameter.is_some());
        writer.write_bytes(&FullBoxHeader::new(version, FullBoxFlags::ZERO).to_bytes())?;
        writer.write_bytes(self.grouping_type.as_bytes())?;
        if let Some(grouping_type_parameter) = self.grouping_type_parameter {
            writer.write_u32(grouping_type_parameter)?;
        }
        write_counted(writer, &self.entries)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, Error, FourCc};

    use super::{SampleToGroupBox, SampleToGroupEntry};
    use crate::encoded;

    /// `sbgp` of version 0 and `grouping_type` `roll`: 2 samples of entry 1, then 3 of no group
    const SBGP_0: &[u8] = b"\0\0\0\x24sbgp\0\0\0\0roll\0\0\0\x02\
        \0\0\0\x02\0\0\0\x01\0\0\0\x03\0\0\0\0";

    /// `sbgp` of version 1 and `grouping_type` `sap `, parameter 5: 4 samples of entry `0x10001`
    const SBGP_1: &[u8] = b"\0\0\0\x20sbgp\x01\0\0\0sap \0\0\0\x05\0\0\0\x01\
        \0\0\0\x04\0\x01\0\x01";

    fn without_parameter() -> SampleToGroupBox {
        SampleToGroupBox {
            grouping_type: FourCc::new(*b"roll"),
            grouping_type_parameter: None,
            entries: vec![
                SampleToGroupEntry {
                    sample_count: 2,
                    group_description_index: 1,
                },
                SampleToGroupEntry {
                    sample_count: 3,
                    group_description_index: 0,
                },
            ],
        }
    }

    fn with_parameter() -> SampleToGroupBox {
        SampleToGroupBox {
            grouping_type: FourCc::new(*b"sap "),
            grouping_type_parameter: Some(5),
            entries: vec![SampleToGroupEntry {
                sample_count: 4,
                group_description_index: 0x10001,
            }],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_in_either_version() {
        assert_eq!(
            (
                SampleToGroupBox::decode(SBGP_0),
                SampleToGroupBox::decode(SBGP_1)
            ),
            (
                Ok((without_parameter(), b"".as_slice())),
                Ok((with_parameter(), b"".as_slice()))
            )
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_the_version_its_parameter_calls_for() {
        assert_eq!(
            (encoded(&without_parameter()), encoded(&with_parameter())),
            (SBGP_0.to_vec(), SBGP_1.to_vec())
        );
    }

    #[test]
    fn a_payload_ending_inside_the_parameter_is_refused() {
        assert_eq!(
            SampleToGroupBox::decode_payload(b"\x01\0\0\0sap \0\0"),
            Err(Error::truncated_payload(12, 10))
        );
    }

    #[test]
    fn a_count_of_more_entries_than_the_payload_holds_is_refused() {
        assert_eq!(
            SampleToGroupBox::decode_payload(b"\0\0\0\0roll\0\0\0\x02\0\0\0\x02\0\0\0\x01"),
            Err(Error::truncated_payload(28, 20))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            SampleToGroupBox::decode_payload(b"\x02\0\0\0roll\0\0\0\0"),
            Err(Error::unsupported_version(2))
        );
    }
}
