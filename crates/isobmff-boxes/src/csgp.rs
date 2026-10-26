//! [`CompactSampleToGroupBox`] and the [`CompactSampleToGroupPattern`]s it holds

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FourCc,
    FullBoxFlags, FullBoxHeader,
};

use crate::full_box::read_version_zero;
use crate::table::{read_entries, table_len, write_entry_count};

/// Flag mask of `index_msb_indicates_fragment_local_description`
const INDEX_MSB_INDICATES_FRAGMENT_LOCAL_DESCRIPTION: u32 = 0x80;

/// Flag mask of `grouping_type_parameter_present`
const GROUPING_TYPE_PARAMETER_PRESENT: u32 = 0x40;

/// Pattern of sample group description indices that a run of samples repeats, one pattern of a [`CompactSampleToGroupBox`]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct CompactSampleToGroupPattern {
    /// Number of samples that use the pattern, repeating it from its start
    /// and stopping where the samples end, at least as many as the pattern
    /// holds indices
    pub sample_count: u32,
    /// `sample_group_description_index` of each sample of the pattern, in
    /// order, as many as `pattern_length` counts
    ///
    /// An index is that of a sample group description entry, from 1, or 0 for
    /// a sample of no group of this type. Where
    /// `index_msb_indicates_fragment_local_description` is set, the most
    /// significant bit of the index, as wide as `index_size_code` makes it,
    /// says whether the entry is that of the track fragment, and is kept here.
    pub sample_group_description_indices: Vec<u32>,
}

/// [`CompactSampleToGroupBox`] (`csgp`), ISO/IEC 14496-12 §8.9.5
///
/// The sample group of each sample of a track or track fragment, of one
/// sample grouping, as runs of samples that each repeat a pattern of sample
/// group description indices. It maps samples as a `SampleToGroupBox` does,
/// more compactly where they repeat a few groups.
///
/// The `flags` hold the width of the fields: `pattern_size_code`,
/// `count_size_code` and `index_size_code` are each 0, 1, 2 or 3, for fields
/// of 4, 8, 16 or 32 bits. Fields of 4 bits pack two to a byte, the earlier
/// in the high half, and the last byte of an odd number of indices is padded
/// with zeros. A 4-bit `pattern_length` with a wider `sample_count`, which
/// §8.9.5.3 forbids, is refused, read or written, with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
/// `count_size_code`. A 4-bit `sample_count` with a wider `pattern_length`,
/// which leaves the pairs off byte boundaries, is not read or written: it is
/// refused with [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue)
/// for `count_size_code`.
///
/// The box is written in the widths it holds. A code past 3, a value wider
/// than its field, a pattern of no index, and a `sample_count` below the
/// number of indices of its pattern are refused when the box is written, with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for the field.
///
/// [`Default`] is the box of `grouping_type` four zero bytes, of no
/// parameter and no pattern, of 8-bit fields.
#[doc(alias = "csgp")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct CompactSampleToGroupBox {
    /// Whether the most significant bit of every index says whether the entry
    /// it names is that of the track fragment, which only a box in a track
    /// fragment may set
    pub index_msb_indicates_fragment_local_description: bool,
    /// Code of the width of every `pattern_length`
    pub pattern_size_code: u8,
    /// Code of the width of every `sample_count`
    pub count_size_code: u8,
    /// Code of the width of every `sample_group_description_index`
    pub index_size_code: u8,
    /// Criterion the samples are grouped by, which links the box to the
    /// `SampleGroupDescriptionBox` of the same `grouping_type`
    pub grouping_type: FourCc,
    /// Sub-type of the grouping, whose meaning the `grouping_type` defines,
    /// which flag `grouping_type_parameter_present` says is there
    pub grouping_type_parameter: Option<u32>,
    /// Patterns, in the order the runs of samples that use them follow
    pub patterns: Vec<CompactSampleToGroupPattern>,
}

impl Default for CompactSampleToGroupBox {
    fn default() -> Self {
        Self {
            index_msb_indicates_fragment_local_description: false,
            pattern_size_code: 1,
            count_size_code: 1,
            index_size_code: 1,
            grouping_type: FourCc::new([0; 4]),
            grouping_type_parameter: None,
            patterns: Vec::new(),
        }
    }
}

/// Returns the width in bits of the fields of `code`, the function `f` of the Syntax
fn field_bits(code: u8) -> u32 {
    4 << code
}

/// Reads a field of 8, 16 or 32 bits
fn read_field(reader: &mut FieldReader<'_>, bits: u32) -> Result<u32, Error> {
    match bits {
        8 => Ok(u32::from(reader.read_bytes::<1>()?[0])),
        16 => Ok(u32::from(reader.read_u16()?)),
        _ => reader.read_u32(),
    }
}

/// Writes `value` as a field of 8, 16 or 32 bits, naming `field` where it is too wide
fn write_field(
    writer: &mut FieldWriter<'_>,
    bits: u32,
    value: u32,
    field: &'static str,
) -> Result<(), Error> {
    let too_wide = |_| Error::invalid_value(field);
    match bits {
        8 => writer.write_bytes(&[u8::try_from(value).map_err(too_wide)?]),
        16 => writer.write_u16(u16::try_from(value).map_err(too_wide)?),
        _ => writer.write_u32(value),
    }
}

/// Returns how many bytes `index_count` indices of `index_bits` each take, saturating at `u64::MAX`
fn indices_len(index_count: u64, index_bits: u32) -> u64 {
    index_count
        .saturating_mul(u64::from(index_bits))
        .div_ceil(8)
}

impl CompactSampleToGroupBox {
    /// Returns the widths in bits of `pattern_length` and `sample_count`, which are both 4 bits or neither
    fn pair_bits(&self) -> Result<(u32, u32), Error> {
        if self.pattern_size_code > 3 {
            return Err(Error::invalid_value("pattern_size_code"));
        }
        if self.count_size_code > 3 {
            return Err(Error::invalid_value("count_size_code"));
        }
        if self.pattern_size_code == 0 && self.count_size_code != 0 {
            return Err(Error::invalid_value("count_size_code"));
        }
        // Why not read a 4-bit `sample_count` after a wider `pattern_length`:
        // such a pair is not a whole number of bytes, and §8.9.5.3 does not
        // say whether the indices then start on a byte; GPAC refuses it too.
        if self.pattern_size_code != 0 && self.count_size_code == 0 {
            return Err(Error::unsupported_value("count_size_code"));
        }

        Ok((
            field_bits(self.pattern_size_code),
            field_bits(self.count_size_code),
        ))
    }
}

impl BoxDefinition for CompactSampleToGroupBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"csgp");
}

impl BoxDecode for CompactSampleToGroupBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let flags = read_version_zero(reader)?.bits();
        let mut compact = Self {
            index_msb_indicates_fragment_local_description: flags
                & INDEX_MSB_INDICATES_FRAGMENT_LOCAL_DESCRIPTION
                != 0,
            pattern_size_code: u8::try_from(flags >> 4 & 0x3).unwrap_or_default(),
            count_size_code: u8::try_from(flags >> 2 & 0x3).unwrap_or_default(),
            index_size_code: u8::try_from(flags & 0x3).unwrap_or_default(),
            grouping_type: FourCc::new(*reader.read_bytes::<4>()?),
            grouping_type_parameter: None,
            patterns: Vec::new(),
        };
        let (pattern_bits, count_bits) = compact.pair_bits()?;
        if flags & GROUPING_TYPE_PARAMETER_PRESENT != 0 {
            compact.grouping_type_parameter = Some(reader.read_u32()?);
        }
        let pattern_count = reader.read_u32()?;
        // Why not read the pairs as one run of bits: a pair of a 4-bit
        // `pattern_length` and a 4-bit `sample_count` fills a byte, and a pair of
        // wider fields fills whole bytes, so every pair starts on a byte.
        let pair_len = u64::from(pattern_bits.saturating_add(count_bits) / 8);
        let pairs = read_entries(reader, pattern_count, pair_len, |reader| {
            if pattern_bits == 4 {
                let [byte] = *reader.read_bytes::<1>()?;
                return Ok((u32::from(byte >> 4), u32::from(byte & 0x0f)));
            }
            Ok((
                read_field(reader, pattern_bits)?,
                read_field(reader, count_bits)?,
            ))
        })?;

        let index_bits = field_bits(compact.index_size_code);
        let index_count = pairs
            .iter()
            .map(|(pattern_length, _)| u64::from(*pattern_length))
            .fold(0, u64::saturating_add);
        let indices_len = indices_len(index_count, index_bits);
        // Why not leave the bound to read_slice: on a target whose usize is
        // narrower than the length of the indices, the conversion below
        // saturates, and the failure would name that length rather than theirs.
        reader.require(indices_len)?;
        let indices = reader.read_slice(usize::try_from(indices_len).unwrap_or(usize::MAX))?;
        let indices: Vec<u32> = match index_bits {
            4 => indices
                .iter()
                .flat_map(|byte| [u32::from(byte >> 4), u32::from(byte & 0x0f)])
                .collect(),
            8 => indices.iter().copied().map(u32::from).collect(),
            16 => indices
                .as_chunks::<2>()
                .0
                .iter()
                .copied()
                .map(u16::from_be_bytes)
                .map(u32::from)
                .collect(),
            _ => indices
                .as_chunks::<4>()
                .0
                .iter()
                .copied()
                .map(u32::from_be_bytes)
                .collect(),
        };
        let mut indices = indices.into_iter();
        compact.patterns = pairs
            .into_iter()
            .map(
                |(pattern_length, sample_count)| CompactSampleToGroupPattern {
                    sample_count,
                    sample_group_description_indices: indices
                        .by_ref()
                        .take(usize::try_from(pattern_length).unwrap_or(usize::MAX))
                        .collect(),
                },
            )
            .collect();

        Ok(compact)
    }
}

impl BoxEncode for CompactSampleToGroupBox {
    fn payload_len(&self) -> u64 {
        let pair_bits = field_bits(self.pattern_size_code.min(3))
            .saturating_add(field_bits(self.count_size_code.min(3)));
        let fields_len = if self.grouping_type_parameter.is_some() {
            4 + 4 + 4 + 4
        } else {
            4 + 4 + 4
        };

        let index_count = self
            .patterns
            .iter()
            .map(|pattern| {
                u64::try_from(pattern.sample_group_description_indices.len()).unwrap_or(u64::MAX)
            })
            .fold(0, u64::saturating_add);

        table_len(self.patterns.len(), u64::from(pair_bits / 8))
            .saturating_add(indices_len(
                index_count,
                field_bits(self.index_size_code.min(3)),
            ))
            .saturating_add(fields_len)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let (pattern_bits, count_bits) = self.pair_bits()?;
        if self.index_size_code > 3 {
            return Err(Error::invalid_value("index_size_code"));
        }
        let index_bits = field_bits(self.index_size_code);
        let flags = [
            (
                self.index_msb_indicates_fragment_local_description,
                INDEX_MSB_INDICATES_FRAGMENT_LOCAL_DESCRIPTION,
            ),
            (
                self.grouping_type_parameter.is_some(),
                GROUPING_TYPE_PARAMETER_PRESENT,
            ),
        ]
        .into_iter()
        .filter(|(set, _)| *set)
        .fold(
            u32::from(self.pattern_size_code) << 4
                | u32::from(self.count_size_code) << 2
                | u32::from(self.index_size_code),
            |flags, (_, mask)| flags | mask,
        );
        let flags = FullBoxFlags::new(flags).unwrap_or(FullBoxFlags::ZERO);
        writer.write_bytes(&FullBoxHeader::new(0, flags).to_bytes())?;
        writer.write_bytes(self.grouping_type.as_bytes())?;
        if let Some(grouping_type_parameter) = self.grouping_type_parameter {
            writer.write_u32(grouping_type_parameter)?;
        }
        write_entry_count(writer, self.patterns.len())?;
        for pattern in &self.patterns {
            let pattern_length =
                u32::try_from(pattern.sample_group_description_indices.len()).unwrap_or(u32::MAX);
            if pattern_length == 0 {
                return Err(Error::invalid_value("pattern_length"));
            }
            if pattern.sample_count < pattern_length {
                return Err(Error::invalid_value("sample_count"));
            }
            if pattern_bits == 4 {
                if pattern_length > 0x0f {
                    return Err(Error::invalid_value("pattern_length"));
                }
                if pattern.sample_count > 0x0f {
                    return Err(Error::invalid_value("sample_count"));
                }
                let [.., byte] = (pattern_length << 4 | pattern.sample_count).to_be_bytes();
                writer.write_bytes(&[byte])?;
                continue;
            }
            write_field(writer, pattern_bits, pattern_length, "pattern_length")?;
            write_field(writer, count_bits, pattern.sample_count, "sample_count")?;
        }

        let mut indices = self
            .patterns
            .iter()
            .flat_map(|pattern| &pattern.sample_group_description_indices)
            .copied();
        if index_bits != 4 {
            return indices.try_for_each(|index| {
                write_field(writer, index_bits, index, "sample_group_description_index")
            });
        }
        while let Some(high) = indices.next() {
            let low = indices.next().unwrap_or_default();
            if high > 0x0f || low > 0x0f {
                return Err(Error::invalid_value("sample_group_description_index"));
            }
            let [.., byte] = (high << 4 | low).to_be_bytes();
            writer.write_bytes(&[byte])?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error, FourCc};

    use super::{CompactSampleToGroupBox, CompactSampleToGroupPattern};
    use crate::encoded;

    /// `csgp` of `grouping_type` `roll` and 8-bit fields: 5 samples repeating entries 1 and 2, then 2 samples of entry 3
    const CSGP_8: &[u8] = b"\0\0\0\x1bcsgp\0\0\0\x15roll\0\0\0\x02\x02\x05\x01\x02\x01\x02\x03";

    /// `csgp` of `grouping_type` `tele` with parameter 7, of 4-bit fields and
    /// fragment-local indices: 3 samples repeating entries 1, 9 and 2, the
    /// last byte padded
    const CSGP_4: &[u8] = b"\0\0\0\x1bcsgp\0\0\0\xc0tele\0\0\0\x07\0\0\0\x01\x33\x19\x20";

    /// `csgp` of `grouping_type` `sap `, of 16-bit pattern lengths, 32-bit
    /// sample counts and 16-bit indices: `0x10000` samples of entry `0x1234`
    const CSGP_WIDE: &[u8] = b"\0\0\0\x1ccsgp\0\0\0\x2esap \0\0\0\x01\0\x01\0\x01\0\0\x12\x34";

    fn pattern(sample_count: u32, indices: &[u32]) -> CompactSampleToGroupPattern {
        CompactSampleToGroupPattern {
            sample_count,
            sample_group_description_indices: indices.to_vec(),
        }
    }

    fn eight_bit() -> CompactSampleToGroupBox {
        CompactSampleToGroupBox {
            grouping_type: FourCc::new(*b"roll"),
            patterns: vec![pattern(5, &[1, 2]), pattern(2, &[3])],
            ..CompactSampleToGroupBox::default()
        }
    }

    fn four_bit() -> CompactSampleToGroupBox {
        CompactSampleToGroupBox {
            index_msb_indicates_fragment_local_description: true,
            pattern_size_code: 0,
            count_size_code: 0,
            index_size_code: 0,
            grouping_type: FourCc::new(*b"tele"),
            grouping_type_parameter: Some(7),
            patterns: vec![pattern(3, &[1, 9, 2])],
        }
    }

    fn wide() -> CompactSampleToGroupBox {
        CompactSampleToGroupBox {
            index_msb_indicates_fragment_local_description: false,
            pattern_size_code: 2,
            count_size_code: 3,
            index_size_code: 2,
            grouping_type: FourCc::new(*b"sap "),
            grouping_type_parameter: None,
            patterns: vec![pattern(0x10000, &[0x1234])],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_in_every_width() {
        assert_eq!(
            [
                CompactSampleToGroupBox::decode(CSGP_8),
                CompactSampleToGroupBox::decode(CSGP_4),
                CompactSampleToGroupBox::decode(CSGP_WIDE)
            ],
            [
                Ok((eight_bit(), b"".as_slice())),
                Ok((four_bit(), b"".as_slice())),
                Ok((wide(), b"".as_slice()))
            ]
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_every_width() {
        assert_eq!(
            [
                encoded(&eight_bit()),
                encoded(&four_bit()),
                encoded(&wide())
            ],
            [CSGP_8.to_vec(), CSGP_4.to_vec(), CSGP_WIDE.to_vec()]
        );
    }

    #[test]
    fn a_payload_ending_inside_the_indices_is_refused() {
        assert_eq!(
            CompactSampleToGroupBox::decode_payload(b"\0\0\0\x15roll\0\0\0\x01\x02\x05\x01"),
            Err(Error::truncated_payload(16, 15))
        );
    }

    #[test]
    fn a_count_of_more_patterns_than_the_payload_holds_is_refused() {
        assert_eq!(
            CompactSampleToGroupBox::decode_payload(b"\0\0\0\x15roll\0\0\0\x03\x02\x05\x01\x02"),
            Err(Error::truncated_payload(18, 16))
        );
    }

    #[test]
    fn a_4_bit_sample_count_after_a_wider_pattern_length_is_not_read() {
        assert_eq!(
            CompactSampleToGroupBox::decode_payload(b"\0\0\0\x10roll\0\0\0\0"),
            Err(Error::unsupported_value("count_size_code"))
        );
    }

    #[test]
    fn a_4_bit_pattern_length_with_a_wider_sample_count_is_refused() {
        assert_eq!(
            CompactSampleToGroupBox::decode_payload(b"\0\0\0\x04roll\0\0\0\0"),
            Err(Error::invalid_value("count_size_code"))
        );
    }

    #[test]
    fn a_value_wider_than_its_field_is_refused_when_written() {
        let mut too_wide = eight_bit();
        too_wide.patterns = vec![pattern(0x100, &[1])];

        assert_eq!(
            too_wide.encode(&mut [0; 64]),
            Err(Error::invalid_value("sample_count"))
        );
    }

    #[test]
    fn a_pattern_of_no_index_is_refused_when_written() {
        let mut empty = eight_bit();
        empty.patterns = vec![pattern(1, &[])];

        assert_eq!(
            empty.encode(&mut [0; 64]),
            Err(Error::invalid_value("pattern_length"))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            CompactSampleToGroupBox::decode_payload(b"\x01\0\0\x15roll\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
