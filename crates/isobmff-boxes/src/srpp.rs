//! [`SrtpProcessBox`], the SRTP algorithms a server applies to the packets of a hint track

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, FourCc, FullBoxFlags, FullBoxHeader, OtherBoxes,
};

/// [`SrtpProcessBox`] (`srpp`), ISO/IEC 14496-12 §9.1.2.2
///
/// The algorithms of encryption and integrity protection that SRTP applies
/// to the packets of an SRTP hint track, or, in an SRTP reception hint track,
/// that were applied to the packets it received. Each is a format identifier
/// of §9.1.2.2, such as `ACM1` or `SHM2`, or four spaces where a process
/// outside the file format chooses it.
///
/// The Syntax leaves the version open, so the box reads any version and
/// writes back the one it holds. The `flags` are read past and written as
/// zero.
///
/// # Children
///
/// The Syntax follows the fields with a `SchemeTypeBox` and a
/// `SchemeInformationBox`, which the box keeps with any other child in
/// [`other_boxes`](Self::other_boxes), in the order they came.
#[doc(alias = "srpp", alias = "SRTPProcessBox")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct SrtpProcessBox {
    /// Version of the box
    pub version: u8,
    /// Algorithm encrypting the RTP packets
    pub encryption_algorithm_rtp: FourCc,
    /// Algorithm encrypting the RTCP packets
    pub encryption_algorithm_rtcp: FourCc,
    /// Algorithm protecting the integrity of the RTP packets, which `ANUL` leaves unprotected
    pub integrity_algorithm_rtp: FourCc,
    /// Algorithm protecting the integrity of the RTCP packets
    pub integrity_algorithm_rtcp: FourCc,
    /// Children the box holds no field for, the `schm` and `schi` of the Syntax among them
    pub other_boxes: OtherBoxes,
}

/// Format identifier of four spaces, which leaves the choice of algorithm to a process outside the file format
const OUTSIDE_THE_FILE_FORMAT: FourCc = FourCc::new(*b"    ");

impl Default for SrtpProcessBox {
    fn default() -> Self {
        Self {
            version: 0,
            encryption_algorithm_rtp: OUTSIDE_THE_FILE_FORMAT,
            encryption_algorithm_rtcp: OUTSIDE_THE_FILE_FORMAT,
            integrity_algorithm_rtp: OUTSIDE_THE_FILE_FORMAT,
            integrity_algorithm_rtcp: OUTSIDE_THE_FILE_FORMAT,
            other_boxes: OtherBoxes::new(),
        }
    }
}

impl BoxDefinition for SrtpProcessBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"srpp");
}

impl BoxDecode for SrtpProcessBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?).version();
        let encryption_algorithm_rtp = FourCc::new(*reader.read_bytes::<4>()?);
        let encryption_algorithm_rtcp = FourCc::new(*reader.read_bytes::<4>()?);
        let integrity_algorithm_rtp = FourCc::new(*reader.read_bytes::<4>()?);
        let integrity_algorithm_rtcp = FourCc::new(*reader.read_bytes::<4>()?);

        Ok(Self {
            version,
            encryption_algorithm_rtp,
            encryption_algorithm_rtcp,
            integrity_algorithm_rtp,
            integrity_algorithm_rtcp,
            other_boxes: OtherBoxes::from(ChildBoxes::decode(reader.take_remainder())?),
        })
    }
}

impl BoxEncode for SrtpProcessBox {
    fn payload_len(&self) -> u64 {
        self.other_boxes
            .as_slice()
            .iter()
            .map(AnyBox::encoded_len)
            .fold(4 + 4 * 4, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxHeader::new(self.version, FullBoxFlags::ZERO).to_bytes())?;
        for algorithm in [
            self.encryption_algorithm_rtp,
            self.encryption_algorithm_rtcp,
            self.integrity_algorithm_rtp,
            self.integrity_algorithm_rtcp,
        ] {
            writer.write_bytes(algorithm.as_bytes())?;
        }
        let mut rest = writer.take_remainder();
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, ChildBoxes, FourCc, OtherBoxes};

    use super::SrtpProcessBox;
    use crate::encoded;

    /// `schm` of the scheme `srtp`, version 1, a box this module does not read
    const SCHM: &[u8] = b"\0\0\0\x14schm\0\0\0\0srtp\0\x01\0\0";

    /// `srpp` of AES in counter mode and HMAC-SHA-1 for RTP, the choice for RTCP left outside the file format, with its `schm`
    const SRPP: &[u8] =
        b"\0\0\0\x30srpp\0\0\0\0ACM1    SHM2    \0\0\0\x14schm\0\0\0\0srtp\0\x01\0\0";

    fn process() -> SrtpProcessBox {
        SrtpProcessBox {
            encryption_algorithm_rtp: FourCc::new(*b"ACM1"),
            integrity_algorithm_rtp: FourCc::new(*b"SHM2"),
            other_boxes: OtherBoxes::from(ChildBoxes::decode(SCHM).unwrap()),
            ..SrtpProcessBox::default()
        }
    }

    #[test]
    fn reads_and_writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            SrtpProcessBox::decode(SRPP),
            Ok((process(), b"".as_slice()))
        );
        assert_eq!(encoded(&process()), SRPP);
    }

    #[test]
    fn a_version_other_than_zero_reads_and_writes_back() {
        let (read, _) =
            SrtpProcessBox::decode(b"\0\0\0\x1csrpp\x01\0\0\0ACM1    SHM2    ").unwrap();

        assert_eq!(read.version, 1);
        assert_eq!(encoded(&read), b"\0\0\0\x1csrpp\x01\0\0\0ACM1    SHM2    ");
    }
}
