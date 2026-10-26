//! [`HintStatisticsBox`], the statistics of what a hint track sends, with [`HintMaxRateBox`] and [`HintPayloadIdBox`]

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::hint_statistics::{
    HintBytesSent64Box, HintBytesSentBox, HintImmediateBytesSentBox, HintLargestPacketBox,
    HintLongestPacketBox, HintMaxRelativeTimeBox, HintMediaBytesSentBox, HintMinRelativeTimeBox,
    HintPacketsSent64Box, HintPacketsSentBox, HintPayloadBytesSent64Box, HintPayloadBytesSentBox,
    HintRepeatedBytesSentBox,
};
use crate::sample_entry::encode_optional;

/// [`HintStatisticsBox`] (`hinf`), ISO/IEC 14496-12 §9.1.5
///
/// Statistics of what a hint track sends, beyond those of its
/// [`HintMediaHeaderBox`](crate::HintMediaHeaderBox), in the
/// [`UserDataBox`](crate::UserDataBox) of the track. Any of the statistics
/// may be absent, and `maxr` may be given once for each period it covers.
///
/// The box writes its children in the order of §9.1.5, whatever order they
/// were read in, `payt` last. Children of other types, and a child of these
/// types that does not read, are kept in [`other_boxes`](Self::other_boxes),
/// and written back after them.
#[doc(alias = "hinf", alias = "hintstatisticsbox")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct HintStatisticsBox {
    /// Bytes sent, including the RTP headers, in 64 bits
    pub trpy: Option<HintBytesSent64Box>,
    /// Packets sent, in 64 bits
    pub nump: Option<HintPacketsSent64Box>,
    /// Bytes sent, not including the RTP headers, in 64 bits
    pub tpyl: Option<HintPayloadBytesSent64Box>,
    /// Bytes sent, including the RTP headers, in 32 bits
    pub totl: Option<HintBytesSentBox>,
    /// Packets sent, in 32 bits
    pub npck: Option<HintPacketsSentBox>,
    /// Bytes sent, not including the RTP headers, in 32 bits
    pub tpay: Option<HintPayloadBytesSentBox>,
    /// Largest number of bytes sent in a period, one for each period, in the order they came
    pub maxr: Vec<HintMaxRateBox>,
    /// Bytes sent from the media tracks
    pub dmed: Option<HintMediaBytesSentBox>,
    /// Bytes sent in immediate mode
    pub dimm: Option<HintImmediateBytesSentBox>,
    /// Bytes in repeated packets
    pub drep: Option<HintRepeatedBytesSentBox>,
    /// Smallest relative transmission time
    pub tmin: Option<HintMinRelativeTimeBox>,
    /// Largest relative transmission time
    pub tmax: Option<HintMaxRelativeTimeBox>,
    /// Largest packet sent
    pub pmax: Option<HintLargestPacketBox>,
    /// Longest packet duration
    pub dmax: Option<HintLongestPacketBox>,
    /// Payload types of the RTP packets, in the order they came
    pub payt: Vec<HintPayloadIdBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for HintStatisticsBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"hinf");
}

impl BoxDecode for HintStatisticsBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            trpy: children.take_zero_or_one_if_decoded()?,
            nump: children.take_zero_or_one_if_decoded()?,
            tpyl: children.take_zero_or_one_if_decoded()?,
            totl: children.take_zero_or_one_if_decoded()?,
            npck: children.take_zero_or_one_if_decoded()?,
            tpay: children.take_zero_or_one_if_decoded()?,
            maxr: children.take_zero_or_more_if_decoded(),
            dmed: children.take_zero_or_one_if_decoded()?,
            dimm: children.take_zero_or_one_if_decoded()?,
            drep: children.take_zero_or_one_if_decoded()?,
            tmin: children.take_zero_or_one_if_decoded()?,
            tmax: children.take_zero_or_one_if_decoded()?,
            pmax: children.take_zero_or_one_if_decoded()?,
            dmax: children.take_zero_or_one_if_decoded()?,
            payt: children.take_zero_or_more_if_decoded(),
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for HintStatisticsBox {
    fn payload_len(&self) -> u64 {
        [
            self.trpy.as_ref().map(BoxEncode::encoded_len),
            self.nump.as_ref().map(BoxEncode::encoded_len),
            self.tpyl.as_ref().map(BoxEncode::encoded_len),
            self.totl.as_ref().map(BoxEncode::encoded_len),
            self.npck.as_ref().map(BoxEncode::encoded_len),
            self.tpay.as_ref().map(BoxEncode::encoded_len),
            self.dmed.as_ref().map(BoxEncode::encoded_len),
            self.dimm.as_ref().map(BoxEncode::encoded_len),
            self.drep.as_ref().map(BoxEncode::encoded_len),
            self.tmin.as_ref().map(BoxEncode::encoded_len),
            self.tmax.as_ref().map(BoxEncode::encoded_len),
            self.pmax.as_ref().map(BoxEncode::encoded_len),
            self.dmax.as_ref().map(BoxEncode::encoded_len),
        ]
        .into_iter()
        .flatten()
        .chain(self.maxr.iter().map(BoxEncode::encoded_len))
        .chain(self.payt.iter().map(BoxEncode::encoded_len))
        .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
        .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = writer.take_remainder();
        rest = encode_optional(rest, self.trpy.as_ref())?;
        rest = encode_optional(rest, self.nump.as_ref())?;
        rest = encode_optional(rest, self.tpyl.as_ref())?;
        rest = encode_optional(rest, self.totl.as_ref())?;
        rest = encode_optional(rest, self.npck.as_ref())?;
        rest = encode_optional(rest, self.tpay.as_ref())?;
        for max_rate in &self.maxr {
            rest = max_rate.encode(rest)?;
        }
        rest = encode_optional(rest, self.dmed.as_ref())?;
        rest = encode_optional(rest, self.dimm.as_ref())?;
        rest = encode_optional(rest, self.drep.as_ref())?;
        rest = encode_optional(rest, self.tmin.as_ref())?;
        rest = encode_optional(rest, self.tmax.as_ref())?;
        rest = encode_optional(rest, self.pmax.as_ref())?;
        rest = encode_optional(rest, self.dmax.as_ref())?;
        for payload_id in &self.payt {
            rest = payload_id.encode(rest)?;
        }
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

/// [`HintMaxRateBox`] (`maxr`), ISO/IEC 14496-12 §9.1.5
///
/// The largest number of bytes, RTP headers included, that a hint track
/// sends in any period of the length it gives.
#[doc(alias = "maxr", alias = "hintmaxrate")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct HintMaxRateBox {
    /// Length of the period, in milliseconds
    pub period: u32,
    /// Largest number of bytes sent in any period of the length `period`
    pub bytes: u32,
}

impl BoxDefinition for HintMaxRateBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"maxr");
}

impl BoxDecode for HintMaxRateBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            period: reader.read_u32()?,
            bytes: reader.read_u32()?,
        })
    }
}

impl BoxEncode for HintMaxRateBox {
    fn payload_len(&self) -> u64 {
        4 + 4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(self.period)?;
        writer.write_u32(self.bytes)
    }
}

/// [`HintPayloadIdBox`] (`payt`), ISO/IEC 14496-12 §9.1.5
///
/// A payload type of the RTP packets a hint track sends, with its `rtpmap`
/// string of SDP, which is kept as found.
///
/// An `rtpmap_string` longer than 255 bytes is refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for `count` when
/// the box is written.
#[doc(alias = "payt", alias = "hintpayloadID")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct HintPayloadIdBox {
    /// Payload type the RTP packets carry
    pub payload_id: u32,
    /// `rtpmap` of the payload type, such as `H264/90000`
    pub rtpmap_string: Vec<u8>,
}

impl BoxDefinition for HintPayloadIdBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"payt");
}

impl BoxDecode for HintPayloadIdBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let payload_id = reader.read_u32()?;
        let [count] = *reader.read_bytes::<1>()?;

        Ok(Self {
            payload_id,
            rtpmap_string: reader.read_slice(usize::from(count))?.to_vec(),
        })
    }
}

impl BoxEncode for HintPayloadIdBox {
    fn payload_len(&self) -> u64 {
        u64::try_from(self.rtpmap_string.len())
            .unwrap_or(u64::MAX)
            .saturating_add(4 + 1)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(self.payload_id)?;
        writer
            .write_bytes(&[u8::try_from(self.rtpmap_string.len())
                .map_err(|_| Error::invalid_value("count"))?])?;
        writer.write_slice(&self.rtpmap_string)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, ChildBoxes, Error, OtherBoxes};

    use super::{HintMaxRateBox, HintPayloadIdBox, HintStatisticsBox};
    use crate::encoded;
    use crate::hint_statistics::{HintBytesSentBox, HintPacketsSent64Box};

    /// `totl` of 312688 bytes
    const TOTL: &[u8] = b"\0\0\0\x0ctotl\0\x04\xc5\x70";

    /// `nump` of 361 packets
    const NUMP: &[u8] = b"\0\0\0\x10nump\0\0\0\0\0\0\x01\x69";

    /// `maxr` of at most 2000 bytes in any 1000 ms
    const MAXR_1000: &[u8] = b"\0\0\0\x10maxr\0\0\x03\xe8\0\0\x07\xd0";

    /// `maxr` of at most 300 bytes in any 100 ms
    const MAXR_100: &[u8] = b"\0\0\0\x10maxr\0\0\0\x64\0\0\x01\x2c";

    /// `payt` of the payload type 96, `H264/90000`
    const PAYT: &[u8] = b"\0\0\0\x17payt\0\0\0\x60\x0aH264/90000";

    /// Statistic of a type this module does not read
    const OTHER: &[u8] = b"\0\0\0\x09xtra\x07";

    fn payload_id() -> HintPayloadIdBox {
        HintPayloadIdBox {
            payload_id: 96,
            rtpmap_string: b"H264/90000".to_vec(),
        }
    }

    #[test]
    fn the_rate_and_the_payload_id_read_and_write_the_bytes_their_syntax_lays_out() {
        let max_rate = HintMaxRateBox {
            period: 1000,
            bytes: 2000,
        };

        assert_eq!(
            HintMaxRateBox::decode(MAXR_1000),
            Ok((max_rate, b"".as_slice()))
        );
        assert_eq!(
            HintPayloadIdBox::decode(PAYT),
            Ok((payload_id(), b"".as_slice()))
        );
        assert_eq!(encoded(&max_rate), MAXR_1000);
        assert_eq!(encoded(&payload_id()), PAYT);
    }

    #[test]
    fn a_payload_id_whose_count_runs_past_the_payload_is_refused() {
        assert_eq!(
            HintPayloadIdBox::decode_payload(b"\0\0\0\x60\x0aH264"),
            Err(Error::truncated_payload(15, 9))
        );
    }

    #[test]
    fn an_rtpmap_string_longer_than_its_count_carries_is_refused_when_written() {
        let payload_id = HintPayloadIdBox {
            rtpmap_string: vec![b'a'; 256],
            ..payload_id()
        };

        assert_eq!(
            payload_id.encode(&mut [0; 269]).map(|_| ()),
            Err(Error::invalid_value("count"))
        );
    }

    #[test]
    fn writes_its_statistics_in_the_order_of_the_subclause_whatever_order_they_were_read_in() {
        let read = HintStatisticsBox::decode_payload(
            &[PAYT, MAXR_1000, OTHER, TOTL, MAXR_100, NUMP].concat(),
        )
        .unwrap();

        assert_eq!(
            read,
            HintStatisticsBox {
                nump: Some(HintPacketsSent64Box { packets_sent: 361 }),
                totl: Some(HintBytesSentBox {
                    bytes_sent: 312_688
                }),
                maxr: vec![
                    HintMaxRateBox {
                        period: 1000,
                        bytes: 2000
                    },
                    HintMaxRateBox {
                        period: 100,
                        bytes: 300
                    },
                ],
                payt: vec![payload_id()],
                other_boxes: OtherBoxes::from(ChildBoxes::decode(OTHER).unwrap()),
                ..HintStatisticsBox::default()
            }
        );
        assert_eq!(
            encoded(&read),
            [
                b"\0\0\0\x64hinf".as_slice(),
                NUMP,
                TOTL,
                MAXR_1000,
                MAXR_100,
                PAYT,
                OTHER
            ]
            .concat()
        );
    }
}
