//! [`HintInformationBox`], the SDP information of a hinted presentation and its hint tracks

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, FourCc, OtherBoxes,
};

/// [`HintInformationBox`] (`hnti`), ISO/IEC 14496-12 §9.1.4
///
/// Holds the fragments of session description a hinter leaves for a server,
/// in the [`UserDataBox`](crate::UserDataBox) of the movie or of a hint
/// track. The Syntax names the box of the movie `moviehintinformation`, which
/// holds an [`RtpMovieHintInformationBox`], and that of a track
/// `trackhintinformation`, which holds an
/// [`RtpTrackSdpHintInformationBox`]; both are `hnti`, so one type reads
/// either.
///
/// The box writes `rtp ` then `sdp `, the order of §9.1.4. Children of other
/// types, the information for protocols other than RTP among them, and a
/// child of these types that does not read, are kept in
/// [`other_boxes`](Self::other_boxes), and written back after them.
#[doc(
    alias = "hnti",
    alias = "moviehintinformation",
    alias = "trackhintinformation"
)]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct HintInformationBox {
    /// Session description of the movie for RTP, in the `hnti` of the movie
    pub rtp: Option<RtpMovieHintInformationBox>,
    /// SDP of an RTP hint track, in the `hnti` of the track
    pub sdp: Option<RtpTrackSdpHintInformationBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for HintInformationBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"hnti");
}

impl BoxDecode for HintInformationBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            rtp: children.take_zero_or_one_if_decoded()?,
            sdp: children.take_zero_or_one_if_decoded()?,
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for HintInformationBox {
    fn payload_len(&self) -> u64 {
        self.rtp
            .iter()
            .map(BoxEncode::encoded_len)
            .chain(self.sdp.iter().map(BoxEncode::encoded_len))
            .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
            .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = writer.take_remainder();
        if let Some(rtp) = &self.rtp {
            rest = rtp.encode(rest)?;
        }
        if let Some(sdp) = &self.sdp {
            rest = sdp.encode(rest)?;
        }
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

/// [`RtpMovieHintInformationBox`] (`rtp `), ISO/IEC 14496-12 §9.1.4.2
///
/// The session description of the movie for RTP, which a server places
/// before the media-specific lines of the description it forms. Only the
/// format `sdp ` is defined, whose text is a series of lines each ended by
/// CRLF. The text is kept as found, to the end of the box.
#[doc(alias = "rtp", alias = "rtpmoviehintinformation")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RtpMovieHintInformationBox {
    /// Format of the description, `sdp `
    pub description_format: FourCc,
    /// Description, in the format `description_format`
    pub sdp_text: Vec<u8>,
}

impl Default for RtpMovieHintInformationBox {
    fn default() -> Self {
        Self {
            description_format: FourCc::new(*b"sdp "),
            sdp_text: Vec::new(),
        }
    }
}

impl BoxDefinition for RtpMovieHintInformationBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"rtp ");
}

impl BoxDecode for RtpMovieHintInformationBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let description_format = FourCc::new(*reader.read_bytes::<4>()?);

        Ok(Self {
            description_format,
            sdp_text: reader.take_remainder().to_vec(),
        })
    }
}

impl BoxEncode for RtpMovieHintInformationBox {
    fn payload_len(&self) -> u64 {
        u64::try_from(self.sdp_text.len())
            .unwrap_or(u64::MAX)
            .saturating_add(4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(self.description_format.as_bytes())?;
        writer.write_slice(&self.sdp_text)
    }
}

/// [`RtpTrackSdpHintInformationBox`] (`sdp `), ISO/IEC 14496-12 §9.1.4.3
///
/// The SDP of an RTP hint track, a series of lines each ended by CRLF, which
/// a server places among the media-specific lines of the description it
/// forms. The text is kept as found, to the end of the box.
#[doc(alias = "sdp", alias = "rtptracksdphintinformation")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct RtpTrackSdpHintInformationBox {
    /// SDP of the track
    pub sdp_text: Vec<u8>,
}

impl BoxDefinition for RtpTrackSdpHintInformationBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"sdp ");
}

impl BoxDecode for RtpTrackSdpHintInformationBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            sdp_text: reader.take_remainder().to_vec(),
        })
    }
}

impl BoxEncode for RtpTrackSdpHintInformationBox {
    fn payload_len(&self) -> u64 {
        u64::try_from(self.sdp_text.len()).unwrap_or(u64::MAX)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_slice(&self.sdp_text)
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, ChildBoxes, OtherBoxes};

    use super::{HintInformationBox, RtpMovieHintInformationBox, RtpTrackSdpHintInformationBox};
    use crate::encoded;

    /// `rtp ` of the SDP line `s=x`
    const RTP: &[u8] = b"\0\0\0\x11rtp sdp s=x\r\n";

    /// `sdp ` of the SDP line `a=control:trackID=1`
    const SDP: &[u8] = b"\0\0\0\x1dsdp a=control:trackID=1\r\n";

    /// Information for a protocol other than RTP, a box this module does not read
    const OTHER: &[u8] = b"\0\0\0\x09xtra\x07";

    fn movie() -> RtpMovieHintInformationBox {
        RtpMovieHintInformationBox {
            sdp_text: b"s=x\r\n".to_vec(),
            ..RtpMovieHintInformationBox::default()
        }
    }

    fn track() -> RtpTrackSdpHintInformationBox {
        RtpTrackSdpHintInformationBox {
            sdp_text: b"a=control:trackID=1\r\n".to_vec(),
        }
    }

    #[test]
    fn the_descriptions_read_and_write_their_text_as_found() {
        assert_eq!(
            RtpMovieHintInformationBox::decode(RTP),
            Ok((movie(), b"".as_slice()))
        );
        assert_eq!(
            RtpTrackSdpHintInformationBox::decode(SDP),
            Ok((track(), b"".as_slice()))
        );
        assert_eq!(encoded(&movie()), RTP);
        assert_eq!(encoded(&track()), SDP);
    }

    #[test]
    fn writes_rtp_then_sdp_then_other_boxes_whatever_order_they_were_read_in() {
        let read = HintInformationBox::decode_payload(&[OTHER, SDP, RTP].concat()).unwrap();

        assert_eq!(
            read,
            HintInformationBox {
                rtp: Some(movie()),
                sdp: Some(track()),
                other_boxes: OtherBoxes::from(ChildBoxes::decode(OTHER).unwrap()),
            }
        );
        assert_eq!(
            encoded(&read),
            [b"\0\0\0\x3fhnti".as_slice(), RTP, SDP, OTHER].concat()
        );
    }
}
