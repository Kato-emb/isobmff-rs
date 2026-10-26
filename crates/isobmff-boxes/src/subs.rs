//! [`SubSampleInformationBox`] and the sub-samples it describes

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFlags,
    FullBoxHeader,
};

use crate::ssrt::SubSampleDescription;
use crate::table::{read_entries, write_entry_count};

/// Properties of one sub-sample, as the version of its [`SubSampleInformationBox`] states them
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SubSampleProperties {
    /// Properties every sub-sample of version 0 or 1 states in full
    Explicit {
        /// Degradation priority of the sub-sample, higher for one of greater
        /// impact on the decoded quality
        subsample_priority: u8,
        /// 0 where the sub-sample is required to decode the sample, 1 where it
        /// is not but may be used for enhancements
        discardable: u8,
        /// Parameters the codec in use defines, 0 where it defines none
        codec_specific_parameters: u32,
    },
    /// Properties of version 2 that are those of an entry of the
    /// `SubSampleReferenceTableBox` of the same `flags`
    Reference {
        /// Index, from 0, of the entry of the `SubSampleReferenceTableBox`
        reference_idx: u16,
    },
    /// Properties of version 2 that the sub-sample states itself
    Description(SubSampleDescription),
}

/// Contiguous range of bytes of a sample, one sub-sample of a [`SubSampleInformationEntry`]
///
/// [`Default`] is the sub-sample of no byte, whose properties are stated in
/// full and are all 0.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SubSample {
    /// Size of the sub-sample, in bytes
    pub subsample_size: u32,
    /// Properties of the sub-sample
    pub properties: SubSampleProperties,
}

impl Default for SubSample {
    fn default() -> Self {
        Self {
            subsample_size: 0,
            properties: SubSampleProperties::Explicit {
                subsample_priority: 0,
                discardable: 0,
                codec_specific_parameters: 0,
            },
        }
    }
}

/// Sample that has sub-sample structure and its sub-samples, one entry of a [`SubSampleInformationBox`]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct SubSampleInformationEntry {
    /// Difference, in decoding order, between the number of the sample and
    /// that of the sample of the entry before
    pub sample_delta: u32,
    /// Sub-samples of the sample, none where it has no sub-sample structure
    pub subsamples: Vec<SubSample>,
}

/// [`SubSampleInformationBox`] (`subs`), ISO/IEC 14496-12 §8.7.7
///
/// The sub-samples of the samples that have sub-sample structure, in a table
/// that numbers each sample by its difference from the one before. What a
/// sub-sample is, and what `flags` mean, are defined for each coding system.
///
/// Versions 0 and 1 state the properties of every sub-sample in full, with
/// 16-bit and 32-bit sizes. Version 2 sizes a sub-sample in 16 or 32 bits as
/// `is_size_16_bits` says, and either refers to the properties of an entry of
/// a `SubSampleReferenceTableBox` or states them itself.
///
/// The box is written in the `version` it holds. A sub-sample whose
/// properties that version does not state is refused when the box is written,
/// with [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for `version`,
/// and a value too wide for its field with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for the field.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{SubSample, SubSampleInformationBox, SubSampleInformationEntry};
/// use isobmff_core::{BoxDecode, BoxEncode};
///
/// // A box is built from its default, field by field
/// let mut subsample = SubSample::default();
/// subsample.subsample_size = 0x10;
/// let mut entry = SubSampleInformationEntry::default();
/// entry.sample_delta = 1;
/// entry.subsamples = vec![subsample];
/// let mut information = SubSampleInformationBox::default();
/// information.entries = vec![entry];
///
/// // It writes the bytes its syntax lays out, and reads back from them
/// let mut buffer = [0; 30];
/// information.encode(&mut buffer).unwrap();
/// assert_eq!(
///     &buffer,
///     b"\0\0\0\x1esubs\0\0\0\0\0\0\0\x01\0\0\0\x01\0\x01\0\x10\0\0\0\0\0\0"
/// );
/// assert_eq!(
///     SubSampleInformationBox::decode(&buffer),
///     Ok((information, b"".as_slice()))
/// );
/// ```
#[doc(alias = "subs")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SubSampleInformationBox {
    /// Version of the box: 0, 1 or 2
    pub version: u8,
    /// Flags of the box, whose meaning the coding system defines, 0 where it
    /// defines none
    pub flags: FullBoxFlags,
    /// Whether `subsample_size` is 16 bits wide rather than 32, under version
    /// 2; `false` under the others
    pub is_size_16_bits: bool,
    /// Whether `reference_idx` is 7 bits wide rather than 15, under version 2;
    /// `false` under the others
    pub is_short_ref: bool,
    /// Samples that have sub-sample structure, in decoding order
    pub entries: Vec<SubSampleInformationEntry>,
}

impl Default for SubSampleInformationBox {
    fn default() -> Self {
        Self {
            version: 0,
            flags: FullBoxFlags::ZERO,
            is_size_16_bits: false,
            is_short_ref: false,
            entries: Vec::new(),
        }
    }
}

impl SubSampleInformationBox {
    /// Returns how many bytes `subsample_size` takes under the version and widths the box states
    fn size_len(&self) -> u64 {
        if self.version == 0 || self.version == 2 && self.is_size_16_bits {
            2
        } else {
            4
        }
    }

    /// Reads a sub-sample under the version and widths the box states
    fn read_subsample(&self, reader: &mut FieldReader<'_>) -> Result<SubSample, Error> {
        let subsample_size = if self.size_len() == 2 {
            u32::from(reader.read_u16()?)
        } else {
            reader.read_u32()?
        };
        let properties = if self.version < 2 {
            let [subsample_priority, discardable] = *reader.read_bytes::<2>()?;
            SubSampleProperties::Explicit {
                subsample_priority,
                discardable,
                codec_specific_parameters: reader.read_u32()?,
            }
        } else {
            let [first] = *reader.read_bytes::<1>()?;
            if first & 0x80 == 0 {
                SubSampleProperties::Description(SubSampleDescription::read(first << 1, reader)?)
            } else if self.is_short_ref {
                SubSampleProperties::Reference {
                    reference_idx: u16::from(first & 0x7f),
                }
            } else {
                let [second] = *reader.read_bytes::<1>()?;
                SubSampleProperties::Reference {
                    reference_idx: u16::from_be_bytes([first & 0x7f, second]),
                }
            }
        };

        Ok(SubSample {
            subsample_size,
            properties,
        })
    }

    /// Writes a sub-sample under the version and widths the box states
    fn write_subsample(
        &self,
        writer: &mut FieldWriter<'_>,
        subsample: &SubSample,
    ) -> Result<(), Error> {
        if self.size_len() == 2 {
            writer.write_u16(
                u16::try_from(subsample.subsample_size)
                    .map_err(|_| Error::invalid_value("subsample_size"))?,
            )?;
        } else {
            writer.write_u32(subsample.subsample_size)?;
        }
        match (self.version, subsample.properties) {
            (
                0 | 1,
                SubSampleProperties::Explicit {
                    subsample_priority,
                    discardable,
                    codec_specific_parameters,
                },
            ) => {
                writer.write_bytes(&[subsample_priority, discardable])?;
                writer.write_u32(codec_specific_parameters)
            }
            (2, SubSampleProperties::Reference { reference_idx }) => {
                let maximum = if self.is_short_ref { 0x7f } else { 0x7fff };
                if reference_idx > maximum {
                    return Err(Error::invalid_value("reference_idx"));
                }
                let [high, low] = reference_idx.to_be_bytes();
                if self.is_short_ref {
                    writer.write_bytes(&[0x80 | low])
                } else {
                    writer.write_bytes(&[0x80 | high, low])
                }
            }
            (2, SubSampleProperties::Description(description)) => {
                writer.write_bytes(&[description.flags()? >> 1])?;
                description.write_fields(writer)
            }
            _ => Err(Error::invalid_value("version")),
        }
    }
}

impl BoxDefinition for SubSampleInformationBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"subs");
}

impl BoxDecode for SubSampleInformationBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let header = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?);
        let version = header.version();
        if version > 2 {
            return Err(Error::unsupported_version(version));
        }
        let entry_count = reader.read_u32()?;
        let (is_size_16_bits, is_short_ref) = if version == 2 {
            let [widths] = *reader.read_bytes::<1>()?;
            (widths & 0x80 != 0, widths & 0x40 != 0)
        } else {
            (false, false)
        };
        let layout = Self {
            version,
            flags: header.flags(),
            is_size_16_bits,
            is_short_ref,
            entries: Vec::new(),
        };
        let properties_minimum_len = if version < 2 { 1 + 1 + 4 } else { 1 };
        let subsample_minimum_len = layout.size_len().saturating_add(properties_minimum_len);
        let entries = read_entries(reader, entry_count, 4 + 2, |reader| {
            let sample_delta = reader.read_u32()?;
            let subsample_count = reader.read_u16()?;
            Ok(SubSampleInformationEntry {
                sample_delta,
                subsamples: read_entries(
                    reader,
                    u32::from(subsample_count),
                    subsample_minimum_len,
                    |reader| layout.read_subsample(reader),
                )?,
            })
        })?;

        Ok(Self { entries, ..layout })
    }
}

impl BoxEncode for SubSampleInformationBox {
    fn payload_len(&self) -> u64 {
        let fields_len = if self.version == 2 { 4 + 4 + 1 } else { 4 + 4 };
        let subsample_len = |subsample: &SubSample| -> u64 {
            let properties_len = match subsample.properties {
                SubSampleProperties::Explicit { .. } => 1 + 1 + 4,
                SubSampleProperties::Reference { .. } if self.is_short_ref => 1,
                SubSampleProperties::Reference { .. } => 2,
                SubSampleProperties::Description(description) => description.encoded_len(),
            };
            properties_len.saturating_add(self.size_len())
        };

        self.entries
            .iter()
            .map(|entry| {
                entry
                    .subsamples
                    .iter()
                    .map(subsample_len)
                    .fold(4 + 2, u64::saturating_add)
            })
            .fold(fields_len, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.version > 2 {
            return Err(Error::unsupported_version(self.version));
        }

        writer.write_bytes(&FullBoxHeader::new(self.version, self.flags).to_bytes())?;
        write_entry_count(writer, self.entries.len())?;
        if self.version == 2 {
            writer.write_bytes(&[
                u8::from(self.is_size_16_bits) << 7 | u8::from(self.is_short_ref) << 6
            ])?;
        }
        for entry in &self.entries {
            writer.write_u32(entry.sample_delta)?;
            writer.write_u16(
                u16::try_from(entry.subsamples.len())
                    .map_err(|_| Error::invalid_value("subsample_count"))?,
            )?;
            for subsample in &entry.subsamples {
                self.write_subsample(writer, subsample)?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error, FullBoxFlags};

    use super::{
        SubSample, SubSampleInformationBox, SubSampleInformationEntry, SubSampleProperties,
    };
    use crate::{SubSampleDescription, encoded};

    /// Sub-sample of version 0 or 1 that stated its properties in full
    fn explicit(subsample_size: u32, discardable: u8) -> SubSample {
        SubSample {
            subsample_size,
            properties: SubSampleProperties::Explicit {
                subsample_priority: 4,
                discardable,
                codec_specific_parameters: 0x0102_0304,
            },
        }
    }

    /// `subs` of version 0 and flags 2: sample 1 of two sub-samples, then sample 3 of none
    const SUBS_0: &[u8] = b"\0\0\0\x2csubs\0\0\0\x02\0\0\0\x02\
        \0\0\0\x01\0\x02\x01\x00\x04\x00\x01\x02\x03\x04\x00\x20\x04\x01\x01\x02\x03\x04\
        \0\0\0\x02\0\0";

    /// The same `subs` in version 1, its sizes 32 bits wide
    const SUBS_1: &[u8] = b"\0\0\0\x30subs\x01\0\0\x02\0\0\0\x02\
        \0\0\0\x01\0\x02\0\0\x01\x00\x04\x00\x01\x02\x03\x04\0\0\x00\x20\x04\x01\x01\x02\x03\x04\
        \0\0\0\x02\0\0";

    fn sub_sample_information(version: u8) -> SubSampleInformationBox {
        SubSampleInformationBox {
            version,
            flags: FullBoxFlags::new(2).unwrap(),
            is_size_16_bits: false,
            is_short_ref: false,
            entries: vec![
                SubSampleInformationEntry {
                    sample_delta: 1,
                    subsamples: vec![explicit(0x100, 0), explicit(0x20, 1)],
                },
                SubSampleInformationEntry {
                    sample_delta: 2,
                    subsamples: vec![],
                },
            ],
        }
    }

    /// `subs` of version 2, 16-bit sizes and 7-bit references: one sample of a
    /// sub-sample that refers to entry 5, and one that states it is
    /// discardable, of priority 7 and of 2 bytes of parameters `0xABCD`
    const SUBS_2_SHORT: &[u8] = b"\0\0\0\x20subs\x02\0\0\0\0\0\0\x01\xc0\
        \0\0\0\x01\0\x02\x01\x00\x85\x00\x20\x74\x07\xab\xcd";

    /// The same sub-samples with 32-bit sizes and 15-bit references
    const SUBS_2_LONG: &[u8] = b"\0\0\0\x25subs\x02\0\0\0\0\0\0\x01\x00\
        \0\0\0\x01\0\x02\0\0\x01\x00\x80\x05\0\0\x00\x20\x74\x07\xab\xcd";

    fn sub_sample_information_2(short: bool) -> SubSampleInformationBox {
        SubSampleInformationBox {
            version: 2,
            flags: FullBoxFlags::ZERO,
            is_size_16_bits: short,
            is_short_ref: short,
            entries: vec![SubSampleInformationEntry {
                sample_delta: 1,
                subsamples: vec![
                    SubSample {
                        subsample_size: 0x100,
                        properties: SubSampleProperties::Reference { reference_idx: 5 },
                    },
                    SubSample {
                        subsample_size: 0x20,
                        properties: SubSampleProperties::Description(SubSampleDescription {
                            discardable: true,
                            subsample_priority: Some(7),
                            csp_length_minus_1: 1,
                            codec_specific_parameters: Some(0xabcd),
                        }),
                    },
                ],
            }],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_in_every_version() {
        assert_eq!(
            [
                SubSampleInformationBox::decode(SUBS_0),
                SubSampleInformationBox::decode(SUBS_1),
                SubSampleInformationBox::decode(SUBS_2_SHORT),
                SubSampleInformationBox::decode(SUBS_2_LONG)
            ],
            [
                Ok((sub_sample_information(0), b"".as_slice())),
                Ok((sub_sample_information(1), b"".as_slice())),
                Ok((sub_sample_information_2(true), b"".as_slice())),
                Ok((sub_sample_information_2(false), b"".as_slice()))
            ]
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_every_version() {
        assert_eq!(
            [
                encoded(&sub_sample_information(0)),
                encoded(&sub_sample_information(1)),
                encoded(&sub_sample_information_2(true)),
                encoded(&sub_sample_information_2(false))
            ],
            [
                SUBS_0.to_vec(),
                SUBS_1.to_vec(),
                SUBS_2_SHORT.to_vec(),
                SUBS_2_LONG.to_vec()
            ]
        );
    }

    #[test]
    fn properties_its_version_does_not_state_are_refused_when_written() {
        let mut mixed = sub_sample_information(0);
        mixed.version = 2;

        assert_eq!(
            mixed.encode(&mut [0; 49]),
            Err(Error::invalid_value("version"))
        );
    }

    #[test]
    fn a_reference_wider_than_7_bits_is_refused_when_written() {
        let mut wide = sub_sample_information_2(true);
        let first = wide.entries.first_mut().unwrap().subsamples.first_mut();
        first.unwrap().properties = SubSampleProperties::Reference {
            reference_idx: 0x80,
        };

        assert_eq!(
            wide.encode(&mut [0; 32]),
            Err(Error::invalid_value("reference_idx"))
        );
    }

    #[test]
    fn a_count_of_more_sub_samples_than_the_payload_holds_is_refused() {
        assert_eq!(
            SubSampleInformationBox::decode_payload(
                b"\0\0\0\0\0\0\0\x01\0\0\0\x01\0\x02\x01\x00\x04\x00\x01\x02\x03\x04"
            ),
            Err(Error::truncated_payload(30, 22))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            SubSampleInformationBox::decode_payload(b"\x03\0\0\0\0\0\0\0"),
            Err(Error::unsupported_version(3))
        );
    }
}
