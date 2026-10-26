//! The TLV boxes of an RTP packet entry: [`RtpOffsetTlvBox`], [`RtpHeaderExtensionTlvBox`] and [`ReceivedCsrcBox`]

use alloc::vec::Vec;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::table::{entries_len, read_to_end, write_entries};

/// [`RtpOffsetTlvBox`] (`rtpo`), ISO/IEC 14496-12 §9.1.3.2
///
/// One of the TLV boxes in the extra information of an RTP packet entry in a
/// hint sample: the offset to the RTP timestamp of the packet, so packets
/// lie in decoding order and carry their presentation timestamps. The
/// subclause defines the box in prose, without a Syntax.
#[doc(alias = "rtpo", alias = "rtpoffsetTLV")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct RtpOffsetTlvBox {
    /// Offset to the RTP timestamp of the packet
    pub offset: i32,
}

impl BoxDefinition for RtpOffsetTlvBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"rtpo");
}

impl BoxDecode for RtpOffsetTlvBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            offset: reader.read_i32()?,
        })
    }
}

impl BoxEncode for RtpOffsetTlvBox {
    fn payload_len(&self) -> u64 {
        4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_i32(self.offset)
    }
}

/// [`RtpHeaderExtensionTlvBox`] (`rtpx`), ISO/IEC 14496-12 §9.4.1.4
///
/// One of the TLV boxes of an RTP packet entry in a reception hint sample:
/// the RTP header extension of the received packet, raw, as the application
/// defines it.
#[doc(alias = "rtpx", alias = "rtphdrextTLV")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct RtpHeaderExtensionTlvBox {
    /// Header extension, as received
    pub data: Vec<u8>,
}

impl BoxDefinition for RtpHeaderExtensionTlvBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"rtpx");
}

impl BoxDecode for RtpHeaderExtensionTlvBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            data: reader.take_remainder().to_vec(),
        })
    }
}

impl BoxEncode for RtpHeaderExtensionTlvBox {
    fn payload_len(&self) -> u64 {
        u64::try_from(self.data.len()).unwrap_or(u64::MAX)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_slice(&self.data)
    }
}

/// [`ReceivedCsrcBox`] (`rcsr`), ISO/IEC 14496-12 §9.4.1.4
///
/// One of the TLV boxes of an RTP packet entry in a reception hint sample:
/// the CSRC fields of the header of the received packet, as many as its CC
/// counts.
#[doc(alias = "rcsr", alias = "receivedCSRC")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct ReceivedCsrcBox {
    /// Contributing source identifiers, in the order of the header
    pub csrc: Vec<u32>,
}

impl BoxDefinition for ReceivedCsrcBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"rcsr");
}

impl BoxDecode for ReceivedCsrcBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            csrc: read_to_end(reader)?,
        })
    }
}

impl BoxEncode for ReceivedCsrcBox {
    fn payload_len(&self) -> u64 {
        entries_len(&self.csrc)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_entries(writer, &self.csrc)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, Error};

    use super::{ReceivedCsrcBox, RtpHeaderExtensionTlvBox, RtpOffsetTlvBox};
    use crate::encoded;

    #[test]
    fn each_box_reads_and_writes_the_bytes_its_definition_lays_out() {
        let rtpo: &[u8] = b"\0\0\0\x0crtpo\xff\xff\xfc\x18";
        let rtpx: &[u8] = b"\0\0\0\x0crtpx\xbe\xde\0\0";
        let rcsr: &[u8] = b"\0\0\0\x10rcsr\0\0\0\x01\0\0\0\x02";
        let csrcs = ReceivedCsrcBox { csrc: vec![1, 2] };
        let extension = RtpHeaderExtensionTlvBox {
            data: vec![0xbe, 0xde, 0, 0],
        };

        assert_eq!(
            RtpOffsetTlvBox::decode(rtpo),
            Ok((RtpOffsetTlvBox { offset: -1000 }, b"".as_slice()))
        );
        assert_eq!(
            RtpHeaderExtensionTlvBox::decode(rtpx),
            Ok((extension.clone(), b"".as_slice()))
        );
        assert_eq!(
            ReceivedCsrcBox::decode(rcsr),
            Ok((csrcs.clone(), b"".as_slice()))
        );
        assert_eq!(encoded(&RtpOffsetTlvBox { offset: -1000 }), rtpo);
        assert_eq!(encoded(&extension), rtpx);
        assert_eq!(encoded(&csrcs), rcsr);
    }

    #[test]
    fn a_csrc_list_ending_inside_an_entry_is_refused() {
        assert_eq!(
            ReceivedCsrcBox::decode_payload(b"\0\0\0\x01\0\0"),
            Err(Error::truncated_payload(8, 6))
        );
    }
}
