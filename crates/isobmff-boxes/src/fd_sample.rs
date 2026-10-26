//! [`FdSample`], a sample of an FD hint track, with its [`ExtraDataBox`] and [`FecInformationBox`]

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes, RawBox,
};

use crate::fdpa::FdPacketBox;

/// [`FdSample`] (`fdsa`), ISO/IEC 14496-12 §9.2.4.1
///
/// A sample of an FD hint track: the packets it generates, in the order a
/// server processes them, and the extra data they need, such as encoding
/// symbols copied into the sample.
///
/// The box writes the `fdpa`s then `extr`, whatever order they were read in.
/// Children of other types, and an `extr` that does not read, are kept in
/// [`other_boxes`](Self::other_boxes), and written back after them.
#[doc(alias = "fdsa", alias = "FDsample")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct FdSample {
    /// Packets of the sample, in the order they came
    pub fdpa: Vec<FdPacketBox>,
    /// Extra data the packets need
    pub extr: Option<ExtraDataBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for FdSample {
    const BOX_TYPE: BoxType = BoxType::compact(*b"fdsa");
}

impl BoxDecode for FdSample {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            fdpa: children.take_zero_or_more()?,
            extr: children.take_zero_or_one_if_decoded()?,
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for FdSample {
    fn payload_len(&self) -> u64 {
        self.fdpa
            .iter()
            .map(BoxEncode::encoded_len)
            .chain(self.extr.iter().map(BoxEncode::encoded_len))
            .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
            .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = writer.take_remainder();
        for packet in &self.fdpa {
            rest = packet.encode(rest)?;
        }
        if let Some(extr) = &self.extr {
            rest = extr.encode(rest)?;
        }
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

/// [`ExtraDataBox`] (`extr`), ISO/IEC 14496-12 §9.2.4.6
///
/// The extra data of an [`FdSample`]: the FEC information of its packets, if
/// it opens with a `feci`, then the bytes the packets copy from, as found.
#[doc(alias = "extr")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct ExtraDataBox {
    /// FEC encoding and payload ID of the packets
    pub feci: Option<FecInformationBox>,
    /// Bytes after the `feci`
    pub extra_data: Vec<u8>,
}

impl BoxDefinition for ExtraDataBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"extr");
}

impl BoxDecode for ExtraDataBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let payload = reader.take_remainder();
        let opens_with_feci = RawBox::split_first(payload)
            .is_ok_and(|(first, _)| first.header().box_type() == FecInformationBox::BOX_TYPE);
        let (feci, extra_data) = if opens_with_feci {
            let (feci, rest) = FecInformationBox::decode(payload)?;
            (Some(feci), rest)
        } else {
            (None, payload)
        };

        Ok(Self {
            feci,
            extra_data: extra_data.to_vec(),
        })
    }
}

impl BoxEncode for ExtraDataBox {
    fn payload_len(&self) -> u64 {
        u64::try_from(self.extra_data.len())
            .unwrap_or(u64::MAX)
            .saturating_add(self.feci.as_ref().map_or(0, BoxEncode::encoded_len))
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = writer.take_remainder();
        if let Some(feci) = &self.feci {
            rest = feci.encode(rest)?;
        }
        let mut writer = FieldWriter::new(rest);
        writer.write_slice(&self.extra_data)?;
        writer.finish()
    }
}

/// [`FecInformationBox`] (`feci`), ISO/IEC 14496-12 §9.2.4.7
///
/// The FEC encoding and the FEC payload ID a server needs to send the
/// packets of an FD sample.
#[doc(alias = "feci", alias = "FECInformationBox")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct FecInformationBox {
    /// FEC encoding scheme, a FEC Encoding ID of IETF RFC 5052
    pub fec_encoding_id: u8,
    /// FEC encoder of an Under-Specified FEC scheme, scoped by `fec_encoding_id`
    pub fec_instance_id: u16,
    /// Source block the encoding symbols of the packets are generated from
    pub source_block_number: u16,
    /// Encoding symbols the packets carry
    pub encoding_symbol_id: u16,
}

impl BoxDefinition for FecInformationBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"feci");
}

impl BoxDecode for FecInformationBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let [fec_encoding_id] = *reader.read_bytes::<1>()?;

        Ok(Self {
            fec_encoding_id,
            fec_instance_id: reader.read_u16()?,
            source_block_number: reader.read_u16()?,
            encoding_symbol_id: reader.read_u16()?,
        })
    }
}

impl BoxEncode for FecInformationBox {
    fn payload_len(&self) -> u64 {
        1 + 2 + 2 + 2
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&[self.fec_encoding_id])?;
        writer.write_u16(self.fec_instance_id)?;
        writer.write_u16(self.source_block_number)?;
        writer.write_u16(self.encoding_symbol_id)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, ChildBoxes, OtherBoxes};

    use super::{ExtraDataBox, FdSample, FecInformationBox};
    use crate::encoded;
    use crate::fdpa::FdPacketBox;

    /// `extr` of the `feci` and two bytes of symbols
    const EXTR: &[u8] = b"\0\0\0\x19extr\0\0\0\x0ffeci\0\0\0\0\x03\0\x04\xab\xcd";

    /// `fdpa` of the object 1, of no constructor
    const FDPA: &[u8] = b"\0\0\0\x0ffdpa\0\0\x01\0\0\0\0";

    /// Box of a type this crate does not read
    const OTHER: &[u8] = b"\0\0\0\x09xtra\x07";

    fn extra_data() -> ExtraDataBox {
        ExtraDataBox {
            feci: Some(FecInformationBox {
                source_block_number: 3,
                encoding_symbol_id: 4,
                ..FecInformationBox::default()
            }),
            extra_data: vec![0xab, 0xcd],
        }
    }

    #[test]
    fn extra_data_reads_and_writes_its_fec_information_then_its_bytes() {
        assert_eq!(
            ExtraDataBox::decode(EXTR),
            Ok((extra_data(), b"".as_slice()))
        );
        assert_eq!(encoded(&extra_data()), EXTR);
    }

    #[test]
    fn extra_data_that_does_not_open_with_fec_information_is_kept_as_its_bytes() {
        let read = ExtraDataBox::decode_payload(b"\x01\x02\x03").unwrap();

        assert_eq!(
            (encoded(&read), read),
            (
                b"\0\0\0\x0bextr\x01\x02\x03".to_vec(),
                ExtraDataBox {
                    feci: None,
                    extra_data: vec![1, 2, 3],
                }
            )
        );
    }

    #[test]
    fn a_sample_writes_its_packets_then_its_extra_data_whatever_order_they_were_read_in() {
        let read = FdSample::decode_payload(&[EXTR, OTHER, FDPA, FDPA].concat()).unwrap();
        let packet = FdPacketBox::decode(FDPA).unwrap().0;

        assert_eq!(
            read,
            FdSample {
                fdpa: vec![packet.clone(), packet],
                extr: Some(extra_data()),
                other_boxes: OtherBoxes::from(ChildBoxes::decode(OTHER).unwrap()),
            }
        );
        assert_eq!(
            encoded(&read),
            [b"\0\0\0\x48fdsa".as_slice(), FDPA, FDPA, EXTR, OTHER].concat()
        );
    }
}
