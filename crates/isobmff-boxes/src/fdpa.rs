//! [`FdPacketBox`], the instructions of an FD hint sample to compose one ALC/LCT or FLUTE packet

use alloc::vec::Vec;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::table::read_entries;

/// [`LctHeaderTemplate`], ISO/IEC 14496-12 §9.2.4.3
///
/// The parts of the LCT header of a packet that a server copies, the rest
/// depending on its own policy. The four `reserved` bits are read past and
/// written as zero.
#[doc(alias = "LCTheaderTemplate")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct LctHeaderTemplate {
    /// Whether the header carries the sender current time
    pub sender_current_time_present: bool,
    /// Whether the header carries the expected residual time
    pub expected_residual_time_present: bool,
    /// Whether the packet ends the session
    pub session_close_bit: bool,
    /// Whether the packet ends the object
    pub object_close_bit: bool,
    /// Transport object identifier (TOI) of the object the packet carries, which a server may change
    pub transport_object_identifier: u16,
}

/// [`LctHeaderExtension`], ISO/IEC 14496-12 §9.2.4.4
///
/// A constructor of one LCT header extension of a packet. An extension of a
/// `header_extension_type` above 127 has a fixed length and holds three bytes
/// of content. Another holds as many 32-bit words as its `length` gives, the
/// first two bytes of which are the type and the length, or no content, a
/// `length` of 0, when the server generates the extension.
///
/// Content of another length than its type allows is refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for `content`
/// when the box is written.
#[doc(alias = "LCTheaderextension")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct LctHeaderExtension {
    /// LCT header extension type (HET)
    pub header_extension_type: u8,
    /// Header extension content (HEC)
    pub content: Vec<u8>,
}

/// Header extension types above this have a fixed length and three bytes of content
const VARIABLE_LENGTH_TYPES: u8 = 127;

impl LctHeaderExtension {
    /// Reads the constructor off the front of `reader`
    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let [header_extension_type] = *reader.read_bytes::<1>()?;
        let content = if header_extension_type > VARIABLE_LENGTH_TYPES {
            reader.read_bytes::<3>()?.to_vec()
        } else {
            let [length] = *reader.read_bytes::<1>()?;
            // Why not `length * 4 - 2` unchecked: a `length` of 0 holds no
            // content, and one of 1 or more is at least the 2 bytes it takes
            // off.
            let content_len = usize::from(length)
                .checked_mul(4)
                .and_then(|words| words.checked_sub(2))
                .unwrap_or(0);
            reader.read_slice(content_len)?.to_vec()
        };

        Ok(Self {
            header_extension_type,
            content,
        })
    }

    /// Returns the `length` of a constructor of a variable length, the 32-bit words the type, the length and the content take, or 0 for no content
    fn length(&self) -> Result<u8, Error> {
        if self.content.is_empty() {
            return Ok(0);
        }
        self.content
            .len()
            .checked_add(2)
            .filter(|bytes| bytes.is_multiple_of(4))
            .and_then(|bytes| u8::try_from(bytes.checked_div(4)?).ok())
            .ok_or_else(|| Error::invalid_value("content"))
    }

    /// Returns the bytes the constructor takes
    fn encoded_len(&self) -> u64 {
        let fields_len = if self.header_extension_type > VARIABLE_LENGTH_TYPES {
            1
        } else {
            1 + 1
        };

        u64::try_from(self.content.len())
            .unwrap_or(u64::MAX)
            .saturating_add(fields_len)
    }

    /// Writes the constructor onto the front of `writer`
    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&[self.header_extension_type])?;
        if self.header_extension_type > VARIABLE_LENGTH_TYPES {
            if self.content.len() != 3 {
                return Err(Error::invalid_value("content"));
            }
        } else {
            writer.write_bytes(&[self.length()?])?;
        }
        writer.write_slice(&self.content)
    }
}

/// [`FdConstructor`], ISO/IEC 14496-12 §9.2.4.5
///
/// A packet constructor, which adds the FEC payload ID or the source or
/// parity symbols to a packet, from the sample, an item, or the XML box of
/// the `meta`. Each takes 16 bytes, its `constructor_type` first, but for
/// [`ItemLarge`](Self::ItemLarge), whose Syntax lays out 20. The pad bytes
/// are read past and written as zero.
///
/// An [`Immediate`](Self::Immediate) of more than 14 bytes is refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for `count`, and
/// a `data_length` of an item wider than its 24 bits with `InvalidValue` for
/// `data_length`, when the box is written.
#[doc(alias = "FDconstructor")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum FdConstructor {
    /// Constructor of type 0, which adds nothing (`FDnoopconstructor`)
    Noop,
    /// Constructor of type 1, which adds the bytes it holds (`FDimmediateconstructor`)
    Immediate {
        /// Bytes added, at most 14
        data: Vec<u8>,
    },
    /// Constructor of type 2, which adds bytes of a sample of a track (`FDsampleconstructor`)
    Sample {
        /// Track the sample is of: 0 for the hint track, -1 for the track the hint track refers to by `hint`, or an index from 1 into its references
        track_ref_index: i8,
        /// Number of bytes added
        length: u16,
        /// Sample the bytes are taken from
        sample_number: u32,
        /// Offset of the bytes in the sample
        sample_offset: u32,
        /// Bytes of a block of compressed samples, 1 for uncompressed samples
        bytes_per_block: u16,
        /// Samples of a block of compressed samples, 1 for uncompressed samples
        samples_per_block: u16,
    },
    /// Constructor of type 3, which adds bytes of an extent of an item (`FDitemconstructor`)
    Item {
        /// Item the bytes are taken from
        item_id: u16,
        /// Index from 1 of the extent of the item
        extent_index: u16,
        /// Offset of the bytes in the extent
        data_offset: u64,
        /// Number of bytes added, 0 for the rest of the extent, in 24 bits
        data_length: u32,
    },
    /// Constructor of type 4, which adds bytes of the `xml ` or `bxml` box of the `meta` (`FDxmlboxconstructor`)
    XmlBox {
        /// Offset of the bytes in the box
        data_offset: u64,
        /// Number of bytes added
        data_length: u32,
    },
    /// Constructor of type 5, which adds bytes of an extent of an item of a 32-bit identifier (`FDitemconstructorLarge`)
    ItemLarge {
        /// Item the bytes are taken from
        item_id: u32,
        /// Index from 1 of the extent of the item
        extent_index: u32,
        /// Offset of the bytes in the extent
        data_offset: u64,
        /// Number of bytes added, 0 for the rest of the extent, in 24 bits
        data_length: u32,
    },
    /// Constructor of a type this crate does not read, with the 15 bytes after its type
    Other {
        /// Type of the constructor
        constructor_type: u8,
        /// Bytes after `constructor_type`
        data: [u8; 15],
    },
}

/// Bytes a constructor takes, but for [`FdConstructor::ItemLarge`]
const CONSTRUCTOR_LEN: u64 = 16;

/// Largest `count` of an [`FdConstructor::Immediate`], which fills the bytes after `constructor_type` and `count`
const IMMEDIATE_CAPACITY: usize = 14;

/// Reads a 24-bit unsigned integer
fn read_u24(reader: &mut FieldReader<'_>) -> Result<u32, Error> {
    let [high, middle, low] = *reader.read_bytes::<3>()?;

    Ok(u32::from_be_bytes([0, high, middle, low]))
}

/// Writes `value` as a 24-bit unsigned integer, refusing one wider than 24 bits with `InvalidValue` for `field`
fn write_u24(writer: &mut FieldWriter<'_>, value: u32, field: &'static str) -> Result<(), Error> {
    let [zero, high, middle, low] = value.to_be_bytes();
    if zero != 0 {
        return Err(Error::invalid_value(field));
    }

    writer.write_bytes(&[high, middle, low])
}

impl FdConstructor {
    /// Reads the constructor off the front of `reader`
    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let [constructor_type] = *reader.read_bytes::<1>()?;
        let constructor = match constructor_type {
            0 => {
                reader.read_bytes::<15>()?;
                Self::Noop
            }
            1 => {
                let [count] = *reader.read_bytes::<1>()?;
                let padded = reader.read_bytes::<14>()?;
                Self::Immediate {
                    data: padded
                        .get(..usize::from(count))
                        .ok_or_else(|| Error::invalid_value("count"))?
                        .to_vec(),
                }
            }
            2 => {
                let [track_ref_index] = *reader.read_bytes::<1>()?;
                Self::Sample {
                    track_ref_index: i8::from_be_bytes([track_ref_index]),
                    length: reader.read_u16()?,
                    sample_number: reader.read_u32()?,
                    sample_offset: reader.read_u32()?,
                    bytes_per_block: reader.read_u16()?,
                    samples_per_block: reader.read_u16()?,
                }
            }
            3 => Self::Item {
                item_id: reader.read_u16()?,
                extent_index: reader.read_u16()?,
                data_offset: reader.read_u64()?,
                data_length: read_u24(reader)?,
            },
            4 => {
                let data_offset = reader.read_u64()?;
                let data_length = reader.read_u32()?;
                reader.read_bytes::<3>()?;
                Self::XmlBox {
                    data_offset,
                    data_length,
                }
            }
            5 => Self::ItemLarge {
                item_id: reader.read_u32()?,
                extent_index: reader.read_u32()?,
                data_offset: reader.read_u64()?,
                data_length: read_u24(reader)?,
            },
            _ => Self::Other {
                constructor_type,
                data: *reader.read_bytes::<15>()?,
            },
        };

        Ok(constructor)
    }

    /// Returns the bytes the constructor takes
    fn encoded_len(&self) -> u64 {
        match self {
            Self::ItemLarge { .. } => 1 + 4 + 4 + 8 + 3,
            Self::Noop
            | Self::Immediate { .. }
            | Self::Sample { .. }
            | Self::Item { .. }
            | Self::XmlBox { .. }
            | Self::Other { .. } => CONSTRUCTOR_LEN,
        }
    }

    /// Writes the constructor onto the front of `writer`
    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        match self {
            Self::Noop => {
                writer.write_bytes(&[0])?;
                writer.write_bytes(&[0; 15])
            }
            Self::Immediate { data } => {
                let count = u8::try_from(data.len())
                    .ok()
                    .filter(|count| usize::from(*count) <= IMMEDIATE_CAPACITY)
                    .ok_or_else(|| Error::invalid_value("count"))?;
                let mut padded = [0; IMMEDIATE_CAPACITY];
                padded
                    .iter_mut()
                    .zip(data)
                    .for_each(|(slot, byte)| *slot = *byte);
                writer.write_bytes(&[1, count])?;
                writer.write_bytes(&padded)
            }
            Self::Sample {
                track_ref_index,
                length,
                sample_number,
                sample_offset,
                bytes_per_block,
                samples_per_block,
            } => {
                writer.write_bytes(&[2])?;
                writer.write_bytes(&track_ref_index.to_be_bytes())?;
                writer.write_u16(*length)?;
                writer.write_u32(*sample_number)?;
                writer.write_u32(*sample_offset)?;
                writer.write_u16(*bytes_per_block)?;
                writer.write_u16(*samples_per_block)
            }
            Self::Item {
                item_id,
                extent_index,
                data_offset,
                data_length,
            } => {
                writer.write_bytes(&[3])?;
                writer.write_u16(*item_id)?;
                writer.write_u16(*extent_index)?;
                writer.write_u64(*data_offset)?;
                write_u24(writer, *data_length, "data_length")
            }
            Self::XmlBox {
                data_offset,
                data_length,
            } => {
                writer.write_bytes(&[4])?;
                writer.write_u64(*data_offset)?;
                writer.write_u32(*data_length)?;
                writer.write_bytes(&[0; 3])
            }
            Self::ItemLarge {
                item_id,
                extent_index,
                data_offset,
                data_length,
            } => {
                writer.write_bytes(&[5])?;
                writer.write_u32(*item_id)?;
                writer.write_u32(*extent_index)?;
                writer.write_u64(*data_offset)?;
                write_u24(writer, *data_length, "data_length")
            }
            Self::Other {
                constructor_type,
                data,
            } => {
                writer.write_bytes(&[*constructor_type])?;
                writer.write_bytes(data)
            }
        }
    }
}

/// [`FdPacketBox`] (`fdpa`), ISO/IEC 14496-12 §9.2.4.2
///
/// One packet of an [`FdSample`](crate::FdSample): the template of its LCT
/// header, the constructors of its LCT header extensions, and the
/// constructors of the FEC payload ID and the symbols it carries.
///
/// More than 65535 constructors of either kind are refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
/// `entrycount1` or `entrycount2` when the box is written.
///
/// # Untrusted input
///
/// The box holds each count against the payload before it reads a
/// constructor, so what it allocates is bounded by the length of the
/// payload, which the caller sets.
#[doc(alias = "fdpa", alias = "FDpacketBox")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct FdPacketBox {
    /// Template of the LCT header of the packet
    pub lct_header_info: LctHeaderTemplate,
    /// Constructors of the LCT header extensions, in order
    pub header_extension_constructors: Vec<LctHeaderExtension>,
    /// Constructors of the FEC payload ID and the symbols, in order
    pub packet_constructors: Vec<FdConstructor>,
}

impl BoxDefinition for FdPacketBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"fdpa");
}

impl BoxDecode for FdPacketBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let [flags] = *reader.read_bytes::<1>()?;
        let lct_header_info = LctHeaderTemplate {
            sender_current_time_present: flags & 0x80 != 0,
            expected_residual_time_present: flags & 0x40 != 0,
            session_close_bit: flags & 0x20 != 0,
            object_close_bit: flags & 0x10 != 0,
            transport_object_identifier: reader.read_u16()?,
        };
        let entrycount1 = reader.read_u16()?;
        let header_extension_constructors =
            read_entries(reader, u32::from(entrycount1), 2, LctHeaderExtension::read)?;
        let entrycount2 = reader.read_u16()?;

        Ok(Self {
            lct_header_info,
            header_extension_constructors,
            packet_constructors: read_entries(
                reader,
                u32::from(entrycount2),
                CONSTRUCTOR_LEN,
                FdConstructor::read,
            )?,
        })
    }
}

impl BoxEncode for FdPacketBox {
    fn payload_len(&self) -> u64 {
        self.header_extension_constructors
            .iter()
            .map(LctHeaderExtension::encoded_len)
            .chain(
                self.packet_constructors
                    .iter()
                    .map(FdConstructor::encoded_len),
            )
            .fold(1 + 2 + 2 + 2, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let template = &self.lct_header_info;
        let flags = [
            (template.sender_current_time_present, 0x80),
            (template.expected_residual_time_present, 0x40),
            (template.session_close_bit, 0x20),
            (template.object_close_bit, 0x10),
        ]
        .into_iter()
        .filter(|(set, _)| *set)
        .fold(0, |flags, (_, bit)| flags | bit);
        writer.write_bytes(&[flags])?;
        writer.write_u16(template.transport_object_identifier)?;
        writer.write_u16(
            u16::try_from(self.header_extension_constructors.len())
                .map_err(|_| Error::invalid_value("entrycount1"))?,
        )?;
        for extension in &self.header_extension_constructors {
            extension.write(writer)?;
        }
        writer.write_u16(
            u16::try_from(self.packet_constructors.len())
                .map_err(|_| Error::invalid_value("entrycount2"))?,
        )?;
        for constructor in &self.packet_constructors {
            constructor.write(writer)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::{FdConstructor, FdPacketBox, LctHeaderExtension, LctHeaderTemplate};
    use crate::encoded;

    /// `fdpa` closing the object 5, with an `EXT_FDT` extension of fixed length and one of two words, an immediate constructor of the FEC payload ID and an item constructor of its symbols
    const FDPA: &[u8] = b"\0\0\0\x3bfdpa\x10\0\x05\0\x02\xc0\x01\x02\x03\x01\x02\xaa\xbb\xcc\xdd\xee\xff\0\x02\x01\x04\0\0\0\x07\0\0\0\0\0\0\0\0\0\0\x03\0\x02\0\x01\0\0\0\0\0\0\x05\x78\0\x05\x78";

    fn packet() -> FdPacketBox {
        FdPacketBox {
            lct_header_info: LctHeaderTemplate {
                object_close_bit: true,
                transport_object_identifier: 5,
                ..LctHeaderTemplate::default()
            },
            header_extension_constructors: vec![
                LctHeaderExtension {
                    header_extension_type: 192,
                    content: vec![1, 2, 3],
                },
                LctHeaderExtension {
                    header_extension_type: 1,
                    content: vec![0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
                },
            ],
            packet_constructors: vec![
                FdConstructor::Immediate {
                    data: vec![0, 0, 0, 7],
                },
                FdConstructor::Item {
                    item_id: 2,
                    extent_index: 1,
                    data_offset: 1400,
                    data_length: 1400,
                },
            ],
        }
    }

    #[test]
    fn reads_and_writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(FdPacketBox::decode(FDPA), Ok((packet(), b"".as_slice())));
        assert_eq!(encoded(&packet()), FDPA);
    }

    #[test]
    fn each_constructor_reads_and_writes_the_bytes_its_syntax_lays_out() {
        let constructors: [(&[u8], FdConstructor); 5] = [
            (b"\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0", FdConstructor::Noop),
            (
                b"\x02\xff\x05\x78\0\0\0\x01\0\0\0\x10\0\x01\0\x01",
                FdConstructor::Sample {
                    track_ref_index: -1,
                    length: 1400,
                    sample_number: 1,
                    sample_offset: 16,
                    bytes_per_block: 1,
                    samples_per_block: 1,
                },
            ),
            (
                b"\x04\0\0\0\0\0\0\0\x20\0\0\x01\0\0\0\0",
                FdConstructor::XmlBox {
                    data_offset: 32,
                    data_length: 256,
                },
            ),
            (
                b"\x05\0\x01\0\0\0\0\0\x01\0\0\0\0\0\0\0\0\0\0\0",
                FdConstructor::ItemLarge {
                    item_id: 0x1_0000,
                    extent_index: 1,
                    data_offset: 0,
                    data_length: 0,
                },
            ),
            (
                b"\x09\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f",
                FdConstructor::Other {
                    constructor_type: 9,
                    data: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
                },
            ),
        ];
        for (bytes, constructor) in constructors {
            let payload = [b"\0\0\x01\0\0\0\x01".as_slice(), bytes].concat();
            let packet = FdPacketBox {
                lct_header_info: LctHeaderTemplate {
                    transport_object_identifier: 1,
                    ..LctHeaderTemplate::default()
                },
                packet_constructors: vec![constructor],
                ..FdPacketBox::default()
            };
            let mut written = vec![0; payload.len()];

            assert_eq!(
                (
                    FdPacketBox::decode_payload(&payload),
                    packet.encode_payload(&mut written).map(|()| written)
                ),
                (Ok(packet), Ok(payload))
            );
        }
    }

    #[test]
    fn a_constructor_whose_bytes_its_type_cannot_hold_is_refused_when_written() {
        let packet = |constructor| FdPacketBox {
            packet_constructors: vec![constructor],
            ..FdPacketBox::default()
        };

        assert_eq!(
            packet(FdConstructor::Immediate { data: vec![0; 15] })
                .encode(&mut [0; 31])
                .map(|_| ()),
            Err(Error::invalid_value("count"))
        );
        assert_eq!(
            packet(FdConstructor::Item {
                item_id: 1,
                extent_index: 1,
                data_offset: 0,
                data_length: 0x0100_0000,
            })
            .encode(&mut [0; 31])
            .map(|_| ()),
            Err(Error::invalid_value("data_length"))
        );
    }

    #[test]
    fn an_extension_whose_content_its_length_cannot_carry_is_refused_when_written() {
        let packet = FdPacketBox {
            header_extension_constructors: vec![LctHeaderExtension {
                header_extension_type: 1,
                content: vec![0; 3],
            }],
            ..FdPacketBox::default()
        };

        assert_eq!(
            packet.encode(&mut [0; 20]).map(|_| ()),
            Err(Error::invalid_value("content"))
        );
    }

    #[test]
    fn an_immediate_constructor_counting_more_than_it_holds_is_refused() {
        assert_eq!(
            FdPacketBox::decode_payload(b"\0\0\x01\0\0\0\x01\x01\x0f\0\0\0\0\0\0\0\0\0\0\0\0\0\0"),
            Err(Error::invalid_value("count"))
        );
    }

    #[test]
    fn a_count_of_more_constructors_than_the_payload_holds_is_refused() {
        assert_eq!(
            FdPacketBox::decode_payload(b"\0\0\x01\0\0\0\x02"),
            Err(Error::truncated_payload(39, 7))
        );
    }
}
