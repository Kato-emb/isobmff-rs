//! The statistics boxes of a [`HintStatisticsBox`](crate::HintStatisticsBox), each one figure of what a hint track sends

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

/// [`HintBytesSent64Box`] (`trpy`), ISO/IEC 14496-12 §9.1.5
///
/// One statistic of a hint track: the 64-bit count of the bytes sent, including the 12-byte RTP headers. [`HintBytesSentBox`] (`totl`) counts the same in 32 bits.
#[doc(alias = "trpy", alias = "hintBytesSent")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct HintBytesSent64Box {
    /// Total bytes sent, including the 12-byte RTP headers
    pub bytes_sent: u64,
}

impl BoxDefinition for HintBytesSent64Box {
    const BOX_TYPE: BoxType = BoxType::compact(*b"trpy");
}

impl BoxDecode for HintBytesSent64Box {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            bytes_sent: reader.read_u64()?,
        })
    }
}

impl BoxEncode for HintBytesSent64Box {
    fn payload_len(&self) -> u64 {
        8
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u64(self.bytes_sent)
    }
}

/// [`HintPacketsSent64Box`] (`nump`), ISO/IEC 14496-12 §9.1.5
///
/// One statistic of a hint track: the 64-bit count of the packets sent. [`HintPacketsSentBox`] (`npck`) counts the same in 32 bits.
#[doc(alias = "nump", alias = "hintPacketsSent")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct HintPacketsSent64Box {
    /// Total packets sent
    pub packets_sent: u64,
}

impl BoxDefinition for HintPacketsSent64Box {
    const BOX_TYPE: BoxType = BoxType::compact(*b"nump");
}

impl BoxDecode for HintPacketsSent64Box {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            packets_sent: reader.read_u64()?,
        })
    }
}

impl BoxEncode for HintPacketsSent64Box {
    fn payload_len(&self) -> u64 {
        8
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u64(self.packets_sent)
    }
}

/// [`HintPayloadBytesSent64Box`] (`tpyl`), ISO/IEC 14496-12 §9.1.5
///
/// One statistic of a hint track: the 64-bit count of the bytes sent, not including the RTP headers. [`HintPayloadBytesSentBox`] (`tpay`) counts the same in 32 bits.
#[doc(alias = "tpyl", alias = "hintBytesSent")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct HintPayloadBytesSent64Box {
    /// Total bytes sent, not including the RTP headers
    pub bytes_sent: u64,
}

impl BoxDefinition for HintPayloadBytesSent64Box {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tpyl");
}

impl BoxDecode for HintPayloadBytesSent64Box {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            bytes_sent: reader.read_u64()?,
        })
    }
}

impl BoxEncode for HintPayloadBytesSent64Box {
    fn payload_len(&self) -> u64 {
        8
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u64(self.bytes_sent)
    }
}

/// [`HintBytesSentBox`] (`totl`), ISO/IEC 14496-12 §9.1.5
///
/// One statistic of a hint track: the 32-bit count of the bytes sent, including the 12-byte RTP headers.
#[doc(alias = "totl", alias = "hintBytesSent")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct HintBytesSentBox {
    /// Total bytes sent, including the 12-byte RTP headers
    pub bytes_sent: u32,
}

impl BoxDefinition for HintBytesSentBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"totl");
}

impl BoxDecode for HintBytesSentBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            bytes_sent: reader.read_u32()?,
        })
    }
}

impl BoxEncode for HintBytesSentBox {
    fn payload_len(&self) -> u64 {
        4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(self.bytes_sent)
    }
}

/// [`HintPacketsSentBox`] (`npck`), ISO/IEC 14496-12 §9.1.5
///
/// One statistic of a hint track: the 32-bit count of the packets sent.
#[doc(alias = "npck", alias = "hintPacketsSent")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct HintPacketsSentBox {
    /// Total packets sent
    pub packets_sent: u32,
}

impl BoxDefinition for HintPacketsSentBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"npck");
}

impl BoxDecode for HintPacketsSentBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            packets_sent: reader.read_u32()?,
        })
    }
}

impl BoxEncode for HintPacketsSentBox {
    fn payload_len(&self) -> u64 {
        4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(self.packets_sent)
    }
}

/// [`HintPayloadBytesSentBox`] (`tpay`), ISO/IEC 14496-12 §9.1.5
///
/// One statistic of a hint track: the 32-bit count of the bytes sent, not including the RTP headers.
#[doc(alias = "tpay", alias = "hintBytesSent")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct HintPayloadBytesSentBox {
    /// Total bytes sent, not including the RTP headers
    pub bytes_sent: u32,
}

impl BoxDefinition for HintPayloadBytesSentBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tpay");
}

impl BoxDecode for HintPayloadBytesSentBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            bytes_sent: reader.read_u32()?,
        })
    }
}

impl BoxEncode for HintPayloadBytesSentBox {
    fn payload_len(&self) -> u64 {
        4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(self.bytes_sent)
    }
}

/// [`HintMediaBytesSentBox`] (`dmed`), ISO/IEC 14496-12 §9.1.5
///
/// One statistic of a hint track: the bytes sent from the media tracks.
#[doc(alias = "dmed", alias = "hintmediaBytesSent")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct HintMediaBytesSentBox {
    /// Total bytes sent from the media tracks
    pub bytes_sent: u64,
}

impl BoxDefinition for HintMediaBytesSentBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"dmed");
}

impl BoxDecode for HintMediaBytesSentBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            bytes_sent: reader.read_u64()?,
        })
    }
}

impl BoxEncode for HintMediaBytesSentBox {
    fn payload_len(&self) -> u64 {
        8
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u64(self.bytes_sent)
    }
}

/// [`HintImmediateBytesSentBox`] (`dimm`), ISO/IEC 14496-12 §9.1.5
///
/// One statistic of a hint track: the bytes sent in immediate mode.
#[doc(alias = "dimm", alias = "hintimmediateBytesSent")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct HintImmediateBytesSentBox {
    /// Total bytes sent in immediate mode
    pub bytes_sent: u64,
}

impl BoxDefinition for HintImmediateBytesSentBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"dimm");
}

impl BoxDecode for HintImmediateBytesSentBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            bytes_sent: reader.read_u64()?,
        })
    }
}

impl BoxEncode for HintImmediateBytesSentBox {
    fn payload_len(&self) -> u64 {
        8
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u64(self.bytes_sent)
    }
}

/// [`HintRepeatedBytesSentBox`] (`drep`), ISO/IEC 14496-12 §9.1.5
///
/// One statistic of a hint track: the bytes in repeated packets.
#[doc(alias = "drep", alias = "hintrepeatedBytesSent")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct HintRepeatedBytesSentBox {
    /// Total bytes in repeated packets
    pub bytes_sent: u64,
}

impl BoxDefinition for HintRepeatedBytesSentBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"drep");
}

impl BoxDecode for HintRepeatedBytesSentBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            bytes_sent: reader.read_u64()?,
        })
    }
}

impl BoxEncode for HintRepeatedBytesSentBox {
    fn payload_len(&self) -> u64 {
        8
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u64(self.bytes_sent)
    }
}

/// [`HintMinRelativeTimeBox`] (`tmin`), ISO/IEC 14496-12 §9.1.5
///
/// One statistic of a hint track: the smallest relative transmission time of a packet.
#[doc(alias = "tmin", alias = "hintminrelativetime")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct HintMinRelativeTimeBox {
    /// Smallest relative transmission time, in milliseconds
    pub time: i32,
}

impl BoxDefinition for HintMinRelativeTimeBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tmin");
}

impl BoxDecode for HintMinRelativeTimeBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            time: reader.read_i32()?,
        })
    }
}

impl BoxEncode for HintMinRelativeTimeBox {
    fn payload_len(&self) -> u64 {
        4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_i32(self.time)
    }
}

/// [`HintMaxRelativeTimeBox`] (`tmax`), ISO/IEC 14496-12 §9.1.5
///
/// One statistic of a hint track: the largest relative transmission time of a packet.
#[doc(alias = "tmax", alias = "hintmaxrelativetime")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct HintMaxRelativeTimeBox {
    /// Largest relative transmission time, in milliseconds
    pub time: i32,
}

impl BoxDefinition for HintMaxRelativeTimeBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tmax");
}

impl BoxDecode for HintMaxRelativeTimeBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            time: reader.read_i32()?,
        })
    }
}

impl BoxEncode for HintMaxRelativeTimeBox {
    fn payload_len(&self) -> u64 {
        4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_i32(self.time)
    }
}

/// [`HintLargestPacketBox`] (`pmax`), ISO/IEC 14496-12 §9.1.5
///
/// One statistic of a hint track: the size of the largest packet sent.
#[doc(alias = "pmax", alias = "hintlargestpacket")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct HintLargestPacketBox {
    /// Size of the largest packet sent, including the RTP header, in bytes
    pub bytes: u32,
}

impl BoxDefinition for HintLargestPacketBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"pmax");
}

impl BoxDecode for HintLargestPacketBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            bytes: reader.read_u32()?,
        })
    }
}

impl BoxEncode for HintLargestPacketBox {
    fn payload_len(&self) -> u64 {
        4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(self.bytes)
    }
}

/// [`HintLongestPacketBox`] (`dmax`), ISO/IEC 14496-12 §9.1.5
///
/// One statistic of a hint track: the longest duration of a packet.
#[doc(alias = "dmax", alias = "hintlongestpacket")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct HintLongestPacketBox {
    /// Longest packet duration, in milliseconds
    pub time: u32,
}

impl BoxDefinition for HintLongestPacketBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"dmax");
}

impl BoxDecode for HintLongestPacketBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            time: reader.read_u32()?,
        })
    }
}

impl BoxEncode for HintLongestPacketBox {
    fn payload_len(&self) -> u64 {
        4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(self.time)
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::BoxDecode;

    use super::{
        HintBytesSent64Box, HintBytesSentBox, HintImmediateBytesSentBox, HintLargestPacketBox,
        HintLongestPacketBox, HintMaxRelativeTimeBox, HintMediaBytesSentBox,
        HintMinRelativeTimeBox, HintPacketsSent64Box, HintPacketsSentBox,
        HintPayloadBytesSent64Box, HintPayloadBytesSentBox, HintRepeatedBytesSentBox,
    };
    use crate::encoded;

    #[test]
    fn the_64_bit_figures_read_and_write_the_bytes_their_syntax_lays_out() {
        let trpy: &[u8] = b"\0\0\0\x10trpy\0\0\0\0\0\x04\xc5\x70";
        let nump: &[u8] = b"\0\0\0\x10nump\0\0\0\0\0\0\x01\x69";
        let tpyl: &[u8] = b"\0\0\0\x10tpyl\0\0\0\0\0\x04\xb4\x84";
        let dmed: &[u8] = b"\0\0\0\x10dmed\0\0\0\x01\0\0\0\0";
        let dimm: &[u8] = b"\0\0\0\x10dimm\0\0\0\0\0\0\0\x02";
        let drep: &[u8] = b"\0\0\0\x10drep\0\0\0\0\0\0\0\x03";

        assert_eq!(
            HintBytesSent64Box::decode(trpy).map(|(read, _)| read.bytes_sent),
            Ok(312_688)
        );
        assert_eq!(
            HintPacketsSent64Box::decode(nump).map(|(read, _)| read.packets_sent),
            Ok(361)
        );
        assert_eq!(
            HintPayloadBytesSent64Box::decode(tpyl).map(|(read, _)| read.bytes_sent),
            Ok(308_356)
        );
        assert_eq!(
            HintMediaBytesSentBox::decode(dmed).map(|(read, _)| read.bytes_sent),
            Ok(0x1_0000_0000)
        );
        assert_eq!(
            HintImmediateBytesSentBox::decode(dimm).map(|(read, _)| read.bytes_sent),
            Ok(2)
        );
        assert_eq!(
            HintRepeatedBytesSentBox::decode(drep).map(|(read, _)| read.bytes_sent),
            Ok(3)
        );
        assert_eq!(
            encoded(&HintBytesSent64Box {
                bytes_sent: 312_688
            }),
            trpy
        );
        assert_eq!(encoded(&HintPacketsSent64Box { packets_sent: 361 }), nump);
        assert_eq!(
            encoded(&HintPayloadBytesSent64Box {
                bytes_sent: 308_356
            }),
            tpyl
        );
        assert_eq!(
            encoded(&HintMediaBytesSentBox {
                bytes_sent: 0x1_0000_0000
            }),
            dmed
        );
        assert_eq!(encoded(&HintImmediateBytesSentBox { bytes_sent: 2 }), dimm);
        assert_eq!(encoded(&HintRepeatedBytesSentBox { bytes_sent: 3 }), drep);
    }

    #[test]
    fn the_32_bit_figures_read_and_write_the_bytes_their_syntax_lays_out() {
        let totl: &[u8] = b"\0\0\0\x0ctotl\0\x04\xc5\x70";
        let npck: &[u8] = b"\0\0\0\x0cnpck\0\0\x01\x69";
        let tpay: &[u8] = b"\0\0\0\x0ctpay\0\x04\xb4\x84";
        let tmin: &[u8] = b"\0\0\0\x0ctmin\xff\xff\xff\xf6";
        let tmax: &[u8] = b"\0\0\0\x0ctmax\0\0\0\x14";
        let pmax: &[u8] = b"\0\0\0\x0cpmax\0\0\x05\xaa";
        let dmax: &[u8] = b"\0\0\0\x0cdmax\0\0\0\x28";

        assert_eq!(
            HintBytesSentBox::decode(totl).map(|(read, _)| read.bytes_sent),
            Ok(312_688)
        );
        assert_eq!(
            HintPacketsSentBox::decode(npck).map(|(read, _)| read.packets_sent),
            Ok(361)
        );
        assert_eq!(
            HintPayloadBytesSentBox::decode(tpay).map(|(read, _)| read.bytes_sent),
            Ok(308_356)
        );
        assert_eq!(
            HintMinRelativeTimeBox::decode(tmin).map(|(read, _)| read.time),
            Ok(-10)
        );
        assert_eq!(
            HintMaxRelativeTimeBox::decode(tmax).map(|(read, _)| read.time),
            Ok(20)
        );
        assert_eq!(
            HintLargestPacketBox::decode(pmax).map(|(read, _)| read.bytes),
            Ok(1450)
        );
        assert_eq!(
            HintLongestPacketBox::decode(dmax).map(|(read, _)| read.time),
            Ok(40)
        );
        assert_eq!(
            encoded(&HintBytesSentBox {
                bytes_sent: 312_688
            }),
            totl
        );
        assert_eq!(encoded(&HintPacketsSentBox { packets_sent: 361 }), npck);
        assert_eq!(
            encoded(&HintPayloadBytesSentBox {
                bytes_sent: 308_356
            }),
            tpay
        );
        assert_eq!(encoded(&HintMinRelativeTimeBox { time: -10 }), tmin);
        assert_eq!(encoded(&HintMaxRelativeTimeBox { time: 20 }), tmax);
        assert_eq!(encoded(&HintLargestPacketBox { bytes: 1450 }), pmax);
        assert_eq!(encoded(&HintLongestPacketBox { time: 40 }), dmax);
    }
}
