//! [`ChannelLayout`], the assignment of the channels of an audio stream

use alloc::vec::Vec;
use core::num::NonZeroU8;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFlags,
    FullBoxHeader,
};

use crate::full_box::write_version_zero;
use crate::table::read_entries;

/// Flag of `stream_structure` stating that the stream carries channels
const CHANNEL_STRUCTURED: u8 = 0x01;

/// Flag of `stream_structure` stating that the stream carries audio objects
const OBJECT_STRUCTURED: u8 = 0x02;

/// `speaker_position` that an explicit `azimuth` and `elevation` follow
const EXPLICIT_POSITION: u8 = 126;

/// Bit of `omitted_channels_present` in the byte it shares with `channel_order_definition`
const OMITTED_CHANNELS_PRESENT: u8 = 0x01;

/// Position of one channel of an explicit layout, an `OutputChannelPosition` of ISO/IEC 23091-3
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SpeakerPosition {
    /// Position ISO/IEC 23091-3 enumerates
    ///
    /// A `speaker_position` of 126, which states an explicit position, is
    /// refused with [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue)
    /// for `speaker_position` when the box is written.
    Enumerated {
        /// `OutputChannelPosition` of ISO/IEC 23091-3
        speaker_position: u8,
    },
    /// Explicit position, `speaker_position` 126
    Explicit {
        /// `LoudspeakerAzimuth` of ISO/IEC 23091-3, in degrees
        azimuth: i16,
        /// `LoudspeakerElevation` of ISO/IEC 23091-3, in degrees
        elevation: i8,
    },
}

impl SpeakerPosition {
    /// Returns the number of bytes the position takes
    const fn encoded_len(self) -> u64 {
        match self {
            Self::Enumerated { .. } => 1,
            Self::Explicit { .. } => 1 + 2 + 1,
        }
    }

    /// Reads the position at the front of `reader`
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends inside the position.
    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let [speaker_position] = *reader.read_bytes::<1>()?;
        if speaker_position != EXPLICIT_POSITION {
            return Ok(Self::Enumerated { speaker_position });
        }
        let azimuth = reader.read_i16()?;
        let [elevation] = *reader.read_bytes::<1>()?;

        Ok(Self::Explicit {
            azimuth,
            elevation: elevation.cast_signed(),
        })
    }

    /// Writes the position at the front of `writer`
    ///
    /// # Errors
    ///
    /// * [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
    ///   `speaker_position`: an enumerated position of 126.
    /// * [`TruncatedBuffer`](isobmff_core::ErrorKind::TruncatedBuffer): the
    ///   buffer ends inside the position.
    fn write(self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        match self {
            Self::Enumerated { speaker_position } if speaker_position == EXPLICIT_POSITION => {
                Err(Error::invalid_value("speaker_position"))
            }
            Self::Enumerated { speaker_position } => writer.write_bytes(&[speaker_position]),
            Self::Explicit { azimuth, elevation } => {
                writer.write_bytes(&[EXPLICIT_POSITION])?;
                writer.write_i16(azimuth)?;
                writer.write_bytes(&[elevation.cast_unsigned()])
            }
        }
    }
}

/// Channel structure of a [`ChannelLayout`]: a layout of ISO/IEC 23091-3, or the position of each channel
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ChannelStructure {
    /// Position of each channel, in the order of the channels: a `definedLayout` of 0
    Explicit {
        /// Position of each channel
        ///
        /// Version 1 counts them in 8 bits, so more than 255 are refused with
        /// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
        /// `layout_channel_count` when the box is written.
        speaker_positions: Vec<SpeakerPosition>,
    },
    /// Layout ISO/IEC 23091-3 defines, all or part of it
    Defined {
        /// `ChannelConfiguration` of ISO/IEC 23091-3
        defined_layout: NonZeroU8,
        /// Where the order of the channels is specified, version 1 alone: 0 by ISO/IEC 23091-3, 1 to 4 by the default order and the orderings #2 to #4 of the audio codec
        ///
        /// Version 0 carries no order, so a value other than 0 is refused
        /// there with [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue)
        /// for `channel_order_definition`, as is one beyond 3 bits in version
        /// 1.
        channel_order_definition: u8,
        /// Channels of the layout absent from the track, bit 0 for the first, [`None`] where the map is not given
        ///
        /// Version 0 always gives the map, so [`None`] is refused there with
        /// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
        /// `omittedChannelsMap`.
        omitted_channels_map: Option<u64>,
    },
}

impl ChannelStructure {
    /// Reads the structure in the version `version`, `definedLayout` included, an explicit layout of version 0 running up to the last `trailing_len` bytes of the payload
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends inside the structure.
    fn read(reader: &mut FieldReader<'_>, version: u8, trailing_len: usize) -> Result<Self, Error> {
        let [defined_layout] = *reader.read_bytes::<1>()?;
        let Some(defined_layout) = NonZeroU8::new(defined_layout) else {
            let speaker_positions = if version == 0 {
                let mut speaker_positions = Vec::new();
                // Why not a count: version 0 leaves it to the sample entry, and
                // each position takes at least one byte, so the loop ends with
                // the payload.
                while reader.remainder().len() > trailing_len {
                    speaker_positions.push(SpeakerPosition::read(reader)?);
                }
                speaker_positions
            } else {
                let [layout_channel_count] = *reader.read_bytes::<1>()?;
                read_entries(
                    reader,
                    u32::from(layout_channel_count),
                    1,
                    SpeakerPosition::read,
                )?
            };
            return Ok(Self::Explicit { speaker_positions });
        };
        if version == 0 {
            return Ok(Self::Defined {
                defined_layout,
                channel_order_definition: 0,
                omitted_channels_map: Some(reader.read_u64()?),
            });
        }
        let [order] = *reader.read_bytes::<1>()?;

        Ok(Self::Defined {
            defined_layout,
            channel_order_definition: (order >> 1) & 0x07,
            omitted_channels_map: if order & OMITTED_CHANNELS_PRESENT == 0 {
                None
            } else {
                Some(reader.read_u64()?)
            },
        })
    }

    /// Returns the number of bytes the structure takes in the version `version`, `definedLayout` included
    fn encoded_len(&self, version: u8) -> u64 {
        match self {
            Self::Explicit { speaker_positions } => speaker_positions
                .iter()
                .map(|position| position.encoded_len())
                .fold(if version == 0 { 1 } else { 1 + 1 }, u64::saturating_add),
            Self::Defined {
                omitted_channels_map,
                ..
            } => {
                if version == 0 {
                    1 + 8
                } else if omitted_channels_map.is_some() {
                    1 + 1 + 8
                } else {
                    1 + 1
                }
            }
        }
    }

    /// Writes the structure in the version `version`, `definedLayout` included
    ///
    /// # Errors
    ///
    /// * [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue): a value the
    ///   version does not carry, as the variants state.
    /// * [`TruncatedBuffer`](isobmff_core::ErrorKind::TruncatedBuffer): the
    ///   buffer ends inside the structure.
    fn write(&self, writer: &mut FieldWriter<'_>, version: u8) -> Result<(), Error> {
        match self {
            Self::Explicit { speaker_positions } => {
                writer.write_bytes(&[0])?;
                if version != 0 {
                    let layout_channel_count = u8::try_from(speaker_positions.len())
                        .map_err(|_| Error::invalid_value("layout_channel_count"))?;
                    writer.write_bytes(&[layout_channel_count])?;
                }
                speaker_positions
                    .iter()
                    .try_for_each(|position| position.write(writer))
            }
            Self::Defined {
                defined_layout,
                channel_order_definition,
                omitted_channels_map,
            } => {
                writer.write_bytes(&[defined_layout.get()])?;
                if version == 0 {
                    if *channel_order_definition != 0 {
                        return Err(Error::invalid_value("channel_order_definition"));
                    }
                    let map =
                        omitted_channels_map.ok_or(Error::invalid_value("omittedChannelsMap"))?;
                    return writer.write_u64(map);
                }
                if *channel_order_definition > 0x07 {
                    return Err(Error::invalid_value("channel_order_definition"));
                }
                let present = if omitted_channels_map.is_some() {
                    OMITTED_CHANNELS_PRESENT
                } else {
                    0
                };
                writer.write_bytes(&[(channel_order_definition << 1) | present])?;
                omitted_channels_map.map_or(Ok(()), |map| writer.write_u64(map))
            }
        }
    }
}

/// [`ChannelLayout`] (`chnl`), ISO/IEC 14496-12 §12.2.4
///
/// Assignment of the channels of the audio stream: whether the stream carries
/// channels, audio objects, both or neither, and the layout of the channels.
/// One variant for each version the box defines.
///
/// The flags of `stream_structure` are whether the structure they announce is
/// there, and its reserved bits are read past and written as zero, as are the
/// `reserved` bits of version 1. The `flags` of the box are read past and
/// written as zero.
///
/// Version 0 does not count the positions of an explicit layout, which the
/// sample entry gives the number of: the box reads positions up to the
/// `object_count` that ends it, or to its end.
#[doc(alias = "chnl")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ChannelLayout {
    /// Version 0
    Version0 {
        /// Channel structure, where the stream carries channels
        channel_structure: Option<ChannelStructure>,
        /// Number of channels that carry audio objects, where the stream carries objects
        object_count: Option<u8>,
    },
    /// Version 1, which conveys the order of the channels and the base channel count of DRC
    Version1 {
        /// Order of the formats from the lowest channel index: 0 unknown, 1 channels first, 2 objects first
        ///
        /// A value beyond 4 bits is refused with
        /// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
        /// `format_ordering` when the box is written.
        format_ordering: u8,
        /// Number of channels of the layout and of audio objects together, the base channel count of DRC
        base_channel_count: u8,
        /// Channel structure, where the stream carries channels
        channel_structure: Option<ChannelStructure>,
        /// Whether the stream carries audio objects, `base_channel_count` less the channels of the layout
        object_structured: bool,
    },
}

impl Default for ChannelLayout {
    fn default() -> Self {
        Self::Version1 {
            format_ordering: 0,
            base_channel_count: 0,
            channel_structure: None,
            object_structured: false,
        }
    }
}

impl BoxDefinition for ChannelLayout {
    const BOX_TYPE: BoxType = BoxType::compact(*b"chnl");
}

/// Returns `flag` if `set`, and otherwise 0
const fn flag_if(set: bool, flag: u8) -> u8 {
    if set { flag } else { 0 }
}

impl BoxDecode for ChannelLayout {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?).version();
        match version {
            0 => {
                let [stream_structure] = *reader.read_bytes::<1>()?;
                let objects = stream_structure & OBJECT_STRUCTURED != 0;
                let channel_structure = if stream_structure & CHANNEL_STRUCTURED == 0 {
                    None
                } else {
                    Some(ChannelStructure::read(reader, 0, usize::from(objects))?)
                };
                let object_count = if objects {
                    Some(reader.read_bytes::<1>()?[0])
                } else {
                    None
                };

                Ok(Self::Version0 {
                    channel_structure,
                    object_count,
                })
            }
            1 => {
                let [structure_and_ordering] = *reader.read_bytes::<1>()?;
                let stream_structure = structure_and_ordering >> 4;
                let [base_channel_count] = *reader.read_bytes::<1>()?;
                let channel_structure = if stream_structure & CHANNEL_STRUCTURED == 0 {
                    None
                } else {
                    Some(ChannelStructure::read(reader, 1, 0)?)
                };

                Ok(Self::Version1 {
                    format_ordering: structure_and_ordering & 0x0f,
                    base_channel_count,
                    channel_structure,
                    object_structured: stream_structure & OBJECT_STRUCTURED != 0,
                })
            }
            _ => Err(Error::unsupported_version(version)),
        }
    }
}

impl BoxEncode for ChannelLayout {
    fn payload_len(&self) -> u64 {
        match self {
            Self::Version0 {
                channel_structure,
                object_count,
            } => channel_structure
                .as_ref()
                .map_or(0, |structure| structure.encoded_len(0))
                .saturating_add(if object_count.is_some() {
                    4 + 1 + 1
                } else {
                    4 + 1
                }),
            Self::Version1 {
                channel_structure, ..
            } => channel_structure
                .as_ref()
                .map_or(0, |structure| structure.encoded_len(1))
                .saturating_add(4 + 1 + 1),
        }
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        match self {
            Self::Version0 {
                channel_structure,
                object_count,
            } => {
                write_version_zero(writer)?;
                writer.write_bytes(&[flag_if(channel_structure.is_some(), CHANNEL_STRUCTURED)
                    | flag_if(object_count.is_some(), OBJECT_STRUCTURED)])?;
                if let Some(structure) = channel_structure {
                    structure.write(writer, 0)?;
                }
                object_count.map_or(Ok(()), |count| writer.write_bytes(&[count]))
            }
            Self::Version1 {
                format_ordering,
                base_channel_count,
                channel_structure,
                object_structured,
            } => {
                if *format_ordering > 0x0f {
                    return Err(Error::invalid_value("format_ordering"));
                }
                let stream_structure = flag_if(channel_structure.is_some(), CHANNEL_STRUCTURED)
                    | flag_if(*object_structured, OBJECT_STRUCTURED);
                writer.write_bytes(&FullBoxHeader::new(1, FullBoxFlags::ZERO).to_bytes())?;
                writer.write_bytes(&[(stream_structure << 4) | format_ordering])?;
                writer.write_bytes(&[*base_channel_count])?;
                channel_structure
                    .as_ref()
                    .map_or(Ok(()), |structure| structure.write(writer, 1))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use core::num::NonZeroU8;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::{ChannelLayout, ChannelStructure, SpeakerPosition};
    use crate::encoded;

    /// `chnl` of version 0 of an explicit layout of a left and a right channel and one at azimuth -90, elevation 30, then 2 audio objects
    const VERSION_0_EXPLICIT: &[u8] = b"\0\0\0\x15chnl\0\0\0\0\x03\0\x02\x01\x7e\xff\xa6\x1e\x02";

    /// `chnl` of version 0 of the layout 2 of ISO/IEC 23091-3, every channel there
    const VERSION_0_DEFINED: &[u8] = b"\0\0\0\x16chnl\0\0\0\0\x01\x02\0\0\0\0\0\0\0\0";

    /// `chnl` of version 1 of 3 base channels: a layout of two enumerated positions, channels first, then audio objects
    const VERSION_1_EXPLICIT: &[u8] = b"\0\0\0\x12chnl\x01\0\0\0\x31\x03\0\x02\x02\x01";

    /// `chnl` of version 1 of 6 base channels: the layout 6, in the default order of the codec, without the channel 3
    const VERSION_1_DEFINED: &[u8] = b"\0\0\0\x18chnl\x01\0\0\0\x10\x06\x06\x03\0\0\0\0\0\0\0\x08";

    fn explicit(speaker_positions: &[SpeakerPosition]) -> Option<ChannelStructure> {
        Some(ChannelStructure::Explicit {
            speaker_positions: speaker_positions.to_vec(),
        })
    }

    fn enumerated(speaker_position: u8) -> SpeakerPosition {
        SpeakerPosition::Enumerated { speaker_position }
    }

    fn layouts() -> [ChannelLayout; 4] {
        [
            ChannelLayout::Version0 {
                channel_structure: explicit(&[
                    enumerated(2),
                    enumerated(1),
                    SpeakerPosition::Explicit {
                        azimuth: -90,
                        elevation: 30,
                    },
                ]),
                object_count: Some(2),
            },
            ChannelLayout::Version0 {
                channel_structure: Some(ChannelStructure::Defined {
                    defined_layout: NonZeroU8::new(2).unwrap(),
                    channel_order_definition: 0,
                    omitted_channels_map: Some(0),
                }),
                object_count: None,
            },
            ChannelLayout::Version1 {
                format_ordering: 1,
                base_channel_count: 3,
                channel_structure: explicit(&[enumerated(2), enumerated(1)]),
                object_structured: true,
            },
            ChannelLayout::Version1 {
                format_ordering: 0,
                base_channel_count: 6,
                channel_structure: Some(ChannelStructure::Defined {
                    defined_layout: NonZeroU8::new(6).unwrap(),
                    channel_order_definition: 1,
                    omitted_channels_map: Some(1 << 3),
                }),
                object_structured: false,
            },
        ]
    }

    const BYTES: [&[u8]; 4] = [
        VERSION_0_EXPLICIT,
        VERSION_0_DEFINED,
        VERSION_1_EXPLICIT,
        VERSION_1_DEFINED,
    ];

    #[test]
    fn reads_the_structure_its_stream_structure_announces_in_either_version() {
        assert_eq!(
            BYTES.map(|bytes| ChannelLayout::decode(bytes).map(|(layout, _)| layout)),
            layouts().map(Ok)
        );
    }

    #[test]
    fn writes_a_stream_structure_announcing_the_structure_it_holds_in_either_version() {
        assert_eq!(
            layouts().map(|layout| encoded(&layout)),
            BYTES.map(<[u8]>::to_vec)
        );
    }

    #[test]
    fn version_1_reads_and_writes_a_layout_without_its_omitted_channels_map() {
        let bytes = b"\0\0\0\x10chnl\x01\0\0\0\x10\x02\x02\x00";
        let layout = ChannelLayout::Version1 {
            format_ordering: 0,
            base_channel_count: 2,
            channel_structure: Some(ChannelStructure::Defined {
                defined_layout: NonZeroU8::new(2).unwrap(),
                channel_order_definition: 0,
                omitted_channels_map: None,
            }),
            object_structured: false,
        };

        assert_eq!(
            (ChannelLayout::decode(bytes), encoded(&layout)),
            (Ok((layout.clone(), b"".as_slice())), bytes.to_vec())
        );
    }

    #[test]
    fn reserved_bits_are_read_past_and_written_as_zero() {
        let (read, _) = ChannelLayout::decode(b"\0\0\0\x0echnl\0\0\0\0\xfe\x05").unwrap();

        assert_eq!(
            (encoded(&read), read),
            (
                b"\0\0\0\x0echnl\0\0\0\0\x02\x05".to_vec(),
                ChannelLayout::Version0 {
                    channel_structure: None,
                    object_count: Some(5)
                }
            )
        );
    }

    #[test]
    fn an_enumerated_position_of_126_is_refused_when_written() {
        let layout = ChannelLayout::Version0 {
            channel_structure: explicit(&[enumerated(126)]),
            object_count: None,
        };

        assert_eq!(
            layout.encode(&mut [0; 15]),
            Err(Error::invalid_value("speaker_position"))
        );
    }

    #[test]
    fn version_0_refuses_what_only_version_1_carries_when_written() {
        let layout = |channel_order_definition, omitted_channels_map| ChannelLayout::Version0 {
            channel_structure: Some(ChannelStructure::Defined {
                defined_layout: NonZeroU8::new(2).unwrap(),
                channel_order_definition,
                omitted_channels_map,
            }),
            object_count: None,
        };

        assert_eq!(
            (
                layout(1, Some(0)).encode(&mut [0; 22]),
                layout(0, None).encode(&mut [0; 22])
            ),
            (
                Err(Error::invalid_value("channel_order_definition")),
                Err(Error::invalid_value("omittedChannelsMap"))
            )
        );
    }

    #[test]
    fn a_format_ordering_beyond_4_bits_is_refused_when_written() {
        let layout = ChannelLayout::Version1 {
            format_ordering: 16,
            base_channel_count: 0,
            channel_structure: None,
            object_structured: false,
        };

        assert_eq!(
            layout.encode(&mut [0; 14]),
            Err(Error::invalid_value("format_ordering"))
        );
    }

    #[test]
    fn a_count_of_positions_the_payload_cannot_hold_is_refused() {
        assert_eq!(
            ChannelLayout::decode_payload(b"\x01\0\0\0\x10\x02\0\x02\x02"),
            Err(Error::truncated_payload(10, 9))
        );
    }

    #[test]
    fn a_version_other_than_0_or_1_is_refused() {
        assert_eq!(
            ChannelLayout::decode_payload(b"\x02\0\0\0\0"),
            Err(Error::unsupported_version(2))
        );
    }

    #[test]
    fn an_explicit_position_ending_inside_its_azimuth_is_refused() {
        assert_eq!(
            ChannelLayout::decode_payload(b"\0\0\0\0\x01\0\x7e\xff"),
            Err(Error::truncated_payload(9, 8))
        );
    }

    #[test]
    fn layouts_of_no_structure_carry_their_flags_alone() {
        assert_eq!(
            vec![
                encoded(&ChannelLayout::Version0 {
                    channel_structure: None,
                    object_count: None
                }),
                encoded(&ChannelLayout::default())
            ],
            vec![
                b"\0\0\0\x0dchnl\0\0\0\0\0".to_vec(),
                b"\0\0\0\x0echnl\x01\0\0\0\0\0".to_vec()
            ]
        );
    }
}
