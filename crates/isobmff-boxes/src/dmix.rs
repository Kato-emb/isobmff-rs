//! [`DownMixInstructions`], the downmixes of an audio stream the production facility controls

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFlags,
    FullBoxHeader,
};

use crate::table::read_entries;

/// Bit of `in_stream` in the byte it shares with `downmix_ID`
const IN_STREAM: u8 = 0x80;

/// Largest value of a field of 7 bits: `downmix_instructions_count`, `targetChannelCount` and `downmix_ID`
const SEVEN_BITS: u8 = 0x7f;

/// Coefficients of a [`DownMixInstruction`] supplied out of the stream, `targetChannelCount` × `baseChannelCount` of them
///
/// The coefficients are in the order of the syntax: for each channel of the
/// target layout, one for each channel of the base layout. One variant for
/// each version of [`DownMixInstructions`], which writes the variant of its
/// version alone and refuses the other with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for the name of
/// its coefficients.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum DownmixCoefficients {
    /// Coefficients of version 0, 4 bits each, coded as Table 9 of ISO/IEC 14496-12 gives them, and Table 10 for an LFE channel
    Version0 {
        /// Code of each coefficient
        bs_downmix_coefficient: Vec<u8>,
    },
    /// Coefficients of version 1, 5 bits each, coded as Table 11 of ISO/IEC 14496-12 gives them
    Version1 {
        /// Offset in dB of every coefficient, 4 bits coded as Table 8 of ISO/IEC 14496-12 gives it
        bs_downmix_offset: u8,
        /// Code of each coefficient
        bs_downmix_coefficient_v1: Vec<u8>,
    },
}

impl DownmixCoefficients {
    /// Returns the number of bytes the coefficients take, padding included
    fn encoded_len(&self) -> u64 {
        let (values, width, offset, _) = self.layout();

        u64::try_from(values.len())
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::from(width))
            .saturating_add(if offset.is_some() { 4 } else { 0 })
            .div_ceil(8)
    }

    /// Returns the coefficients, the number of bits each takes, the bits before them and the bits of padding after them
    fn layout(&self) -> (&[u8], u8, Option<u8>, &'static str) {
        match self {
            Self::Version0 {
                bs_downmix_coefficient,
            } => (bs_downmix_coefficient, 4, None, "bs_downmix_coefficient"),
            Self::Version1 {
                bs_downmix_offset,
                bs_downmix_coefficient_v1,
            } => (
                bs_downmix_coefficient_v1,
                5,
                Some(*bs_downmix_offset),
                "bs_downmix_coefficient_v1",
            ),
        }
    }
}

/// One downmix of [`DownMixInstructions`]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct DownMixInstruction {
    /// Layout after the downmix, a `ChannelConfiguration` of ISO/IEC 23091-3
    pub target_layout: u8,
    /// Number of channels after the downmix, 7 bits, at most the `channelcount` of the sample entry
    pub target_channel_count: u8,
    /// Identifier of the downmix, 7 bits, unique among the downmixes of the sample entry; 0 and `0x7f` are reserved
    pub downmix_id: u8,
    /// Coefficients supplied here, or [`None`] where they are in the stream
    pub downmix_coefficients: Option<DownmixCoefficients>,
}

/// [`DownMixInstructions`] (`dmix`), ISO/IEC 14496-12 §12.2.5
///
/// Downmixes of the audio stream, each to a target layout, with their
/// coefficients where they are not in the stream. Version 0 holds one
/// downmix, and version 1 up to 127 of them.
///
/// The coefficients of a downmix number its `targetChannelCount` times the
/// `baseChannelCount` of the sample entry, which the box itself does not
/// carry: the [`AudioSampleEntry`](crate::AudioSampleEntry) or
/// [`AudioSampleEntryV1`](crate::AudioSampleEntryV1) holding the box reads it
/// with that count. The box writes the coefficients it holds, so each
/// downmix of one box holds a multiple of its `target_channel_count` of
/// them, the same multiple for all: a box that holds otherwise is refused
/// with [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
/// `baseChannelCount` when it is written.
///
/// A field wider than its bits, a version 0 holding other than one downmix,
/// and a version 1 holding more than 127, are refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for the field when
/// the box is written. The `reserved` bits are read past and written as zero,
/// and the padding after the coefficients is written as the version states
/// it: four bits of 1 in version 0, zero bits in version 1. The `flags` are
/// read past and written as zero.
#[doc(alias = "dmix")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct DownMixInstructions {
    /// Version of the box, 0 or 1
    ///
    /// A version other than 0 or 1 is refused with
    /// [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion)
    /// when the box is written.
    pub version: u8,
    /// Downmixes, in the order they came
    pub downmix_instructions: Vec<DownMixInstruction>,
}

impl BoxDefinition for DownMixInstructions {
    const BOX_TYPE: BoxType = BoxType::compact(*b"dmix");
}

/// Returns `version` if the box reads it
///
/// # Errors
///
/// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the
///   version is neither 0 nor 1.
fn supported(version: u8) -> Result<u8, Error> {
    match version {
        0 | 1 => Ok(version),
        _ => Err(Error::unsupported_version(version)),
    }
}

/// Returns the `count` values of `width` bits that `bytes` packs from its most significant bit on, after `skipped_bits`
fn unpack(bytes: &[u8], skipped_bits: usize, width: usize, count: usize) -> Vec<u8> {
    (0..count)
        .map(|index| {
            (0..width).fold(0, |value, bit| {
                let position = skipped_bits
                    .saturating_add(index.saturating_mul(width))
                    .saturating_add(bit);
                let byte = bytes.get(position / 8).copied().unwrap_or(0);
                (value << 1) | ((byte << (position % 8)) >> 7)
            })
        })
        .collect()
}

impl DownMixInstructions {
    /// Reads the payload of a box of a sample entry whose `baseChannelCount` is `base_channel_count`
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends inside a downmix.
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion):
    ///   the version is neither 0 nor 1.
    pub(crate) fn decode_payload(base_channel_count: u16, payload: &[u8]) -> Result<Self, Error> {
        let mut reader = FieldReader::new(payload);
        let version = supported(FullBoxHeader::from_bytes(reader.read_bytes::<4>()?).version())?;
        let count = if version == 0 {
            1
        } else {
            reader.read_bytes::<1>()?[0] & SEVEN_BITS
        };
        let downmix_instructions = read_entries(&mut reader, u32::from(count), 3, |reader| {
            let [target_layout, target_channel_count, in_stream_and_id] =
                *reader.read_bytes::<3>()?;
            let target_channel_count = target_channel_count & SEVEN_BITS;
            let coefficient_count =
                usize::from(target_channel_count).saturating_mul(usize::from(base_channel_count));
            let downmix_coefficients = if in_stream_and_id & IN_STREAM != 0 {
                None
            } else if version == 0 {
                let bytes = reader.read_slice(coefficient_count.div_ceil(2))?;
                Some(DownmixCoefficients::Version0 {
                    bs_downmix_coefficient: unpack(bytes, 0, 4, coefficient_count),
                })
            } else {
                let bits = coefficient_count.saturating_mul(5).saturating_add(4);
                let bytes = reader.read_slice(bits.div_ceil(8))?;
                Some(DownmixCoefficients::Version1 {
                    bs_downmix_offset: bytes.first().map_or(0, |byte| byte >> 4),
                    bs_downmix_coefficient_v1: unpack(bytes, 4, 5, coefficient_count),
                })
            };
            Ok(DownMixInstruction {
                target_layout,
                target_channel_count,
                downmix_id: in_stream_and_id & SEVEN_BITS,
                downmix_coefficients,
            })
        })?;

        Ok(Self {
            version,
            downmix_instructions,
        })
    }
}

/// Writes `coefficients`, `width` bits each after the four bits of `offset` if any, and pads them to a byte with the bits of `padding`
///
/// # Errors
///
/// * [`TruncatedBuffer`](isobmff_core::ErrorKind::TruncatedBuffer): the
///   buffer ends inside the coefficients.
fn pack(
    writer: &mut FieldWriter<'_>,
    offset: Option<u8>,
    coefficients: &[u8],
    width: u8,
    padding: u8,
) -> Result<(), Error> {
    let mut accumulated: u16 = 0;
    let mut bits: u8 = 0;
    let fields = offset
        .map(|offset| (offset, 4))
        .into_iter()
        .chain(coefficients.iter().map(|coefficient| (*coefficient, width)));
    for (value, value_width) in fields {
        accumulated = (accumulated << value_width) | u16::from(value);
        bits = bits.saturating_add(value_width);
        if bits >= 8 {
            bits = bits.saturating_sub(8);
            let [_, byte] = (accumulated >> bits).to_be_bytes();
            writer.write_bytes(&[byte])?;
        }
    }
    if bits > 0 {
        let [_, byte] = ((accumulated << 8_u8.saturating_sub(bits)) | (u16::from(padding) >> bits))
            .to_be_bytes();
        writer.write_bytes(&[byte])?;
    }

    Ok(())
}

impl BoxEncode for DownMixInstructions {
    fn payload_len(&self) -> u64 {
        self.downmix_instructions
            .iter()
            .map(|instruction| {
                instruction
                    .downmix_coefficients
                    .as_ref()
                    .map_or(0, DownmixCoefficients::encoded_len)
                    .saturating_add(3)
            })
            .fold(
                if self.version == 0 { 4 } else { 4 + 1 },
                u64::saturating_add,
            )
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let version = supported(self.version)?;
        let count = u8::try_from(self.downmix_instructions.len())
            .ok()
            .filter(|count| {
                if version == 0 {
                    *count == 1
                } else {
                    *count <= SEVEN_BITS
                }
            })
            .ok_or(Error::invalid_value("downmix_instructions_count"))?;
        let mut base_channel_count = None;
        for instruction in &self.downmix_instructions {
            if instruction.target_channel_count > SEVEN_BITS {
                return Err(Error::invalid_value("targetChannelCount"));
            }
            if instruction.downmix_id > SEVEN_BITS {
                return Err(Error::invalid_value("downmix_ID"));
            }
            let Some(coefficients) = &instruction.downmix_coefficients else {
                continue;
            };
            let (values, width, offset, field) = coefficients.layout();
            if (version == 0) != offset.is_none()
                || offset.is_some_and(|offset| offset > 0x0f)
                || values.iter().any(|value| value >> width != 0)
            {
                return Err(Error::invalid_value(field));
            }
            let target_channel_count = usize::from(instruction.target_channel_count);
            match values.len().checked_div(target_channel_count) {
                Some(per_target)
                    if per_target.checked_mul(target_channel_count) == Some(values.len())
                        && *base_channel_count.get_or_insert(per_target) == per_target => {}
                None if values.is_empty() => {}
                _ => return Err(Error::invalid_value("baseChannelCount")),
            }
        }

        writer.write_bytes(&FullBoxHeader::new(version, FullBoxFlags::ZERO).to_bytes())?;
        if version != 0 {
            writer.write_bytes(&[count])?;
        }
        for instruction in &self.downmix_instructions {
            let in_stream = if instruction.downmix_coefficients.is_none() {
                IN_STREAM
            } else {
                0
            };
            writer.write_bytes(&[
                instruction.target_layout,
                instruction.target_channel_count,
                in_stream | instruction.downmix_id,
            ])?;
            if let Some(coefficients) = &instruction.downmix_coefficients {
                let (values, width, offset, _) = coefficients.layout();
                pack(
                    writer,
                    offset,
                    values,
                    width,
                    if version == 0 { 0xff } else { 0 },
                )?;
            }
        }

        Ok(())
    }
}

/// Payload of a `dmix` of a version [`DownMixInstructions`] reads, left to read until the `baseChannelCount` of its sample entry is known
pub(crate) struct UndecodedDownMixInstructions(pub(crate) Vec<u8>);

impl BoxDefinition for UndecodedDownMixInstructions {
    const BOX_TYPE: BoxType = DownMixInstructions::BOX_TYPE;
}

impl BoxDecode for UndecodedDownMixInstructions {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let payload = reader.take_remainder();
        supported(FieldReader::new(payload).read_bytes::<1>()?[0])?;

        Ok(Self(payload.to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxEncode, Error};

    use super::{DownMixInstruction, DownMixInstructions, DownmixCoefficients};
    use crate::encoded;

    /// `dmix` of version 0 from 3 base channels to the layout 1 of 1 channel, downmix 5: -3 dB, -3 dB, -∞ dB, then four bits of padding
    const VERSION_0: &[u8] = b"\0\0\0\x11dmix\0\0\0\0\x01\x01\x05\x66\xff";

    /// `dmix` of version 1 of two downmixes from 2 base channels: downmix 1 to 1 channel at an offset of 1, 0 dB and -6 dB; downmix 2 in the stream
    const VERSION_1: &[u8] = b"\0\0\0\x15dmix\x01\0\0\0\x02\x01\x01\x01\x12\xc4\x01\x01\x82";

    fn version_0() -> DownMixInstructions {
        DownMixInstructions {
            version: 0,
            downmix_instructions: vec![DownMixInstruction {
                target_layout: 1,
                target_channel_count: 1,
                downmix_id: 5,
                downmix_coefficients: Some(DownmixCoefficients::Version0 {
                    bs_downmix_coefficient: vec![6, 6, 0xf],
                }),
            }],
        }
    }

    fn version_1() -> DownMixInstructions {
        DownMixInstructions {
            version: 1,
            downmix_instructions: vec![
                DownMixInstruction {
                    target_layout: 1,
                    target_channel_count: 1,
                    downmix_id: 1,
                    downmix_coefficients: Some(DownmixCoefficients::Version1 {
                        bs_downmix_offset: 1,
                        bs_downmix_coefficient_v1: vec![0x05, 0x11],
                    }),
                },
                DownMixInstruction {
                    target_layout: 1,
                    target_channel_count: 1,
                    downmix_id: 2,
                    downmix_coefficients: None,
                },
            ],
        }
    }

    #[test]
    fn reads_the_coefficients_of_the_base_channel_count_of_its_sample_entry_in_either_version() {
        assert_eq!(
            (
                DownMixInstructions::decode_payload(3, VERSION_0.get(8..).unwrap()),
                DownMixInstructions::decode_payload(2, VERSION_1.get(8..).unwrap())
            ),
            (Ok(version_0()), Ok(version_1()))
        );
    }

    #[test]
    fn writes_the_coefficients_it_holds_padded_as_its_version_states_in_either_version() {
        assert_eq!(
            (encoded(&version_0()), encoded(&version_1())),
            (VERSION_0.to_vec(), VERSION_1.to_vec())
        );
    }

    #[test]
    fn a_payload_ending_inside_the_coefficients_is_refused() {
        assert_eq!(
            DownMixInstructions::decode_payload(6, VERSION_0.get(8..).unwrap()),
            Err(Error::truncated_payload(10, 9))
        );
    }

    #[test]
    fn a_version_other_than_0_or_1_is_refused() {
        assert_eq!(
            DownMixInstructions::decode_payload(2, b"\x02\0\0\0\0"),
            Err(Error::unsupported_version(2))
        );
    }

    #[test]
    fn coefficients_of_two_base_channel_counts_in_one_box_are_refused_when_written() {
        let mut instructions = version_1();
        instructions
            .downmix_instructions
            .get_mut(1)
            .unwrap()
            .downmix_coefficients = Some(DownmixCoefficients::Version1 {
            bs_downmix_offset: 0,
            bs_downmix_coefficient_v1: vec![0x05, 0x05, 0x05],
        });

        assert_eq!(
            instructions.encode(&mut [0; 32]),
            Err(Error::invalid_value("baseChannelCount"))
        );
    }

    #[test]
    fn coefficients_of_the_other_version_are_refused_when_written() {
        let instructions = DownMixInstructions {
            version: 1,
            ..version_0()
        };

        assert_eq!(
            instructions.encode(&mut [0; 32]),
            Err(Error::invalid_value("bs_downmix_coefficient"))
        );
    }

    #[test]
    fn version_0_of_other_than_one_downmix_is_refused_when_written() {
        let instructions = DownMixInstructions {
            version: 0,
            ..version_1()
        };

        assert_eq!(
            instructions.encode(&mut [0; 32]),
            Err(Error::invalid_value("downmix_instructions_count"))
        );
    }

    #[test]
    fn a_coefficient_wider_than_its_bits_is_refused_when_written() {
        let mut instructions = version_0();
        instructions
            .downmix_instructions
            .get_mut(0)
            .unwrap()
            .downmix_coefficients = Some(DownmixCoefficients::Version0 {
            bs_downmix_coefficient: vec![0x10],
        });

        assert_eq!(
            instructions.encode(&mut [0; 32]),
            Err(Error::invalid_value("bs_downmix_coefficient"))
        );
    }
}
