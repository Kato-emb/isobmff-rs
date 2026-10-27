//! The boxes of the additional data of a hint sample entry: its timing, and the SSRC and cryptographic context of the stream it received

use alloc::vec::Vec;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

/// [`TimescaleEntryBox`] (`tims`), ISO/IEC 14496-12 §9.1.2.1
///
/// The timescale of the RTP timestamps of the packets a hint track forms or
/// received, or of the send times of the packets of an FD hint track.
#[doc(alias = "tims", alias = "timescaleentry")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct TimescaleEntryBox {
    /// Number of time units in a second
    pub timescale: u32,
}

impl BoxDefinition for TimescaleEntryBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tims");
}

impl BoxDecode for TimescaleEntryBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            timescale: reader.read_u32()?,
        })
    }
}

impl BoxEncode for TimescaleEntryBox {
    fn payload_len(&self) -> u64 {
        4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(self.timescale)
    }
}

/// [`TimeOffsetBox`] (`tsro`), ISO/IEC 14496-12 §9.1.2.1
///
/// The offset a server adds to the RTP timestamps in place of a random one,
/// or, in a reception hint track, the first RTP timestamp of the stream in
/// reception order, which reads as 0 where the box is absent.
#[doc(alias = "tsro", alias = "timeoffset")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct TimeOffsetBox {
    /// Offset to the RTP timestamps
    pub offset: i32,
}

impl BoxDefinition for TimeOffsetBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tsro");
}

impl BoxDecode for TimeOffsetBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            offset: reader.read_i32()?,
        })
    }
}

impl BoxEncode for TimeOffsetBox {
    fn payload_len(&self) -> u64 {
        4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_i32(self.offset)
    }
}

/// [`SequenceOffsetBox`] (`snro`), ISO/IEC 14496-12 §9.1.2.1
///
/// The offset a server adds to the RTP sequence numbers in place of a random
/// one.
#[doc(alias = "snro", alias = "sequenceoffset")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct SequenceOffsetBox {
    /// Offset to the RTP sequence numbers
    pub offset: i32,
}

impl BoxDefinition for SequenceOffsetBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"snro");
}

impl BoxDecode for SequenceOffsetBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            offset: reader.read_i32()?,
        })
    }
}

impl BoxEncode for SequenceOffsetBox {
    fn payload_len(&self) -> u64 {
        4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_i32(self.offset)
    }
}

/// [`TimestampSynchronyBox`] (`tssy`), ISO/IEC 14496-12 §9.4.1.2
///
/// Whether the RTP timestamps of an RTP reception hint track are synchronized
/// with those of the other RTP reception hint tracks, which reads as 0 where
/// the box is absent.
///
/// The six `reserved` bits are read past and written as zero. A
/// `timestamp_sync` wider than its two bits is refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
/// `timestamp_sync` when the box is written.
#[doc(alias = "tssy", alias = "timestampsynchrony")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct TimestampSynchronyBox {
    /// 0 when the timestamps may or may not be synchronized, 1 when they are as received, 2 when they are synchronized; 3 is reserved
    pub timestamp_sync: u8,
}

impl BoxDefinition for TimestampSynchronyBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tssy");
}

impl BoxDecode for TimestampSynchronyBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let [byte] = *reader.read_bytes::<1>()?;

        Ok(Self {
            timestamp_sync: byte & 0b11,
        })
    }
}

impl BoxEncode for TimestampSynchronyBox {
    fn payload_len(&self) -> u64 {
        1
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.timestamp_sync > 0b11 {
            return Err(Error::invalid_value("timestamp_sync"));
        }

        writer.write_bytes(&[self.timestamp_sync])
    }
}

/// [`ReceivedSsrcBox`] (`rssr`), ISO/IEC 14496-12 §9.4.1.2
///
/// The SSRC in the header of every packet the entry of an RTP or SRTP
/// reception hint track describes.
#[doc(alias = "rssr")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct ReceivedSsrcBox {
    /// Synchronization source identifier of the packets
    pub ssrc: u32,
}

impl BoxDefinition for ReceivedSsrcBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"rssr");
}

impl BoxDecode for ReceivedSsrcBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            ssrc: reader.read_u32()?,
        })
    }
}

impl BoxEncode for ReceivedSsrcBox {
    fn payload_len(&self) -> u64 {
        4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(self.ssrc)
    }
}

/// [`ReceivedCryptoContextIdBox`] (`ccid`), ISO/IEC 14496-12 §9.4.3.2.2
///
/// The destination port and IP address of the SRTP session an SRTP reception
/// hint track received its packets over, from which the cryptographic
/// context of the stream is recovered.
///
/// The Syntax gives `destIP` 32 bits for an `ip_version` of 4 and 64 bits for
/// one of 6, an IPv6 address being 128 bits, and none for another version.
/// The box keeps the bytes after `ip_version` as found, so an address of
/// either width reads and writes back.
#[doc(alias = "ccid")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct ReceivedCryptoContextIdBox {
    /// Destination port of the session
    pub dest_port: u16,
    /// IP version of the session, 4 or 6
    pub ip_version: u8,
    /// Destination IP address of the session, the bytes after `ip_version`
    pub dest_ip: Vec<u8>,
}

impl BoxDefinition for ReceivedCryptoContextIdBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"ccid");
}

impl BoxDecode for ReceivedCryptoContextIdBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let dest_port = reader.read_u16()?;
        let [ip_version] = *reader.read_bytes::<1>()?;

        Ok(Self {
            dest_port,
            ip_version,
            dest_ip: reader.take_remainder().to_vec(),
        })
    }
}

impl BoxEncode for ReceivedCryptoContextIdBox {
    fn payload_len(&self) -> u64 {
        u64::try_from(self.dest_ip.len())
            .unwrap_or(u64::MAX)
            .saturating_add(2 + 1)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u16(self.dest_port)?;
        writer.write_bytes(&[self.ip_version])?;
        writer.write_slice(&self.dest_ip)
    }
}

/// [`RolloverCounterBox`] (`sroc`), ISO/IEC 14496-12 §9.4.3.2.3
///
/// The rollover counter (ROC) of every SRTP packet the entry of an SRTP
/// reception hint track describes, which decrypting a packet needs.
#[doc(alias = "sroc")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct RolloverCounterBox {
    /// Rollover counter of the packets
    pub rollover_counter: u32,
}

impl BoxDefinition for RolloverCounterBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"sroc");
}

impl BoxDecode for RolloverCounterBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            rollover_counter: reader.read_u32()?,
        })
    }
}

impl BoxEncode for RolloverCounterBox {
    fn payload_len(&self) -> u64 {
        4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(self.rollover_counter)
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::{
        ReceivedCryptoContextIdBox, ReceivedSsrcBox, RolloverCounterBox, SequenceOffsetBox,
        TimeOffsetBox, TimescaleEntryBox, TimestampSynchronyBox,
    };
    use crate::encoded;

    #[test]
    fn each_box_reads_and_writes_the_bytes_its_syntax_lays_out() {
        let tims: &[u8] = b"\0\0\0\x0ctims\0\x01\x5f\x90";
        let tsro: &[u8] = b"\0\0\0\x0ctsro\xff\xff\xff\xfe";
        let snro: &[u8] = b"\0\0\0\x0csnro\0\0\0\x07";
        let rssr: &[u8] = b"\0\0\0\x0crssr\x12\x34\x56\x78";
        let sroc: &[u8] = b"\0\0\0\x0csroc\0\0\0\x02";

        assert_eq!(
            TimescaleEntryBox::decode(tims),
            Ok((TimescaleEntryBox { timescale: 90000 }, b"".as_slice()))
        );
        assert_eq!(
            TimeOffsetBox::decode(tsro),
            Ok((TimeOffsetBox { offset: -2 }, b"".as_slice()))
        );
        assert_eq!(
            SequenceOffsetBox::decode(snro),
            Ok((SequenceOffsetBox { offset: 7 }, b"".as_slice()))
        );
        assert_eq!(
            ReceivedSsrcBox::decode(rssr),
            Ok((ReceivedSsrcBox { ssrc: 0x1234_5678 }, b"".as_slice()))
        );
        assert_eq!(
            RolloverCounterBox::decode(sroc),
            Ok((
                RolloverCounterBox {
                    rollover_counter: 2
                },
                b"".as_slice()
            ))
        );
        assert_eq!(encoded(&TimescaleEntryBox { timescale: 90000 }), tims);
        assert_eq!(encoded(&TimeOffsetBox { offset: -2 }), tsro);
        assert_eq!(encoded(&SequenceOffsetBox { offset: 7 }), snro);
        assert_eq!(encoded(&ReceivedSsrcBox { ssrc: 0x1234_5678 }), rssr);
        assert_eq!(
            encoded(&RolloverCounterBox {
                rollover_counter: 2
            }),
            sroc
        );
    }

    #[test]
    fn timestamp_synchrony_reads_past_its_reserved_bits_and_writes_them_as_zero() {
        let (read, _) = TimestampSynchronyBox::decode(b"\0\0\0\x09tssy\xfe").unwrap();

        assert_eq!(read, TimestampSynchronyBox { timestamp_sync: 2 });
        assert_eq!(encoded(&read), b"\0\0\0\x09tssy\x02");
    }

    #[test]
    fn a_timestamp_sync_wider_than_its_bits_is_refused_when_written() {
        assert_eq!(
            TimestampSynchronyBox { timestamp_sync: 4 }
                .encode(&mut [0; 9])
                .map(|_| ()),
            Err(Error::invalid_value("timestamp_sync"))
        );
    }

    #[test]
    fn a_crypto_context_keeps_the_address_as_found_whatever_its_width() {
        let ipv4: &[u8] = b"\0\0\0\x0fccid\x13\x88\x04\xc0\xa8\0\x01";
        let ipv6: &[u8] = b"\0\0\0\x1bccid\x13\x88\x06\x20\x01\x0d\xb8\0\0\0\0\0\0\0\0\0\0\0\x01";
        let context = |ip_version, dest_ip: &[u8]| ReceivedCryptoContextIdBox {
            dest_port: 5000,
            ip_version,
            dest_ip: dest_ip.to_vec(),
        };

        assert_eq!(
            ReceivedCryptoContextIdBox::decode(ipv4),
            Ok((context(4, &[192, 168, 0, 1]), b"".as_slice()))
        );
        assert_eq!(
            encoded(&ReceivedCryptoContextIdBox::decode(ipv6).unwrap().0),
            ipv6
        );
        assert_eq!(
            ReceivedCryptoContextIdBox::decode_payload(b"\x13\x88"),
            Err(Error::truncated_payload(3, 2))
        );
        assert_eq!(context(4, &[0; 4]).payload_len(), 7);
    }
}
