//! A container box of a crate of its own, implemented on this crate's public items alone

#[cfg(test)]
mod tests {
    use isobmff_core::{
        AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, BoxVariants, ChildBoxWriter,
        ChildBoxes, Error, FieldReader, FieldWriter, OtherBoxes, RawBox,
    };

    /// Box of a vendor's own whose payload is one 16-bit level
    #[derive(PartialEq, Eq, Debug)]
    struct LevelBox {
        level: u16,
    }

    impl BoxDefinition for LevelBox {
        const BOX_TYPE: BoxType = BoxType::compact(*b"vlvl");
    }

    impl BoxDecode for LevelBox {
        fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
            Ok(Self {
                level: reader.read_u16()?,
            })
        }
    }

    impl BoxEncode for LevelBox {
        fn payload_len(&self) -> u64 {
            2
        }

        fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
            writer.write_u16(self.level)
        }
    }

    /// Box of a vendor's own stating a sequence number in 32 bits
    #[derive(PartialEq, Eq, Debug)]
    struct WideSequenceNumberBox {
        sequence_number: u32,
    }

    impl BoxDefinition for WideSequenceNumberBox {
        const BOX_TYPE: BoxType = BoxType::compact(*b"vsq4");
    }

    impl BoxDecode for WideSequenceNumberBox {
        fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
            Ok(Self {
                sequence_number: reader.read_u32()?,
            })
        }
    }

    impl BoxEncode for WideSequenceNumberBox {
        fn payload_len(&self) -> u64 {
            4
        }

        fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
            writer.write_u32(self.sequence_number)
        }
    }

    /// Box of a vendor's own stating a sequence number in 16 bits
    #[derive(PartialEq, Eq, Debug)]
    struct NarrowSequenceNumberBox {
        sequence_number: u16,
    }

    impl BoxDefinition for NarrowSequenceNumberBox {
        const BOX_TYPE: BoxType = BoxType::compact(*b"vsq2");
    }

    impl BoxDecode for NarrowSequenceNumberBox {
        fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
            Ok(Self {
                sequence_number: reader.read_u16()?,
            })
        }
    }

    impl BoxEncode for NarrowSequenceNumberBox {
        fn payload_len(&self) -> u64 {
            2
        }

        fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
            writer.write_u16(self.sequence_number)
        }
    }

    /// Sequence number stated by either variant
    #[derive(PartialEq, Eq, Debug)]
    enum SequenceNumber {
        Wide(WideSequenceNumberBox),
        Narrow(NarrowSequenceNumberBox),
    }

    impl BoxVariants for SequenceNumber {
        const VARIANTS: &'static [BoxType] = &[
            WideSequenceNumberBox::BOX_TYPE,
            NarrowSequenceNumberBox::BOX_TYPE,
        ];

        fn decode_variant(child: RawBox<'_>) -> Result<Self, Error> {
            if child.header().box_type() == WideSequenceNumberBox::BOX_TYPE {
                WideSequenceNumberBox::decode_payload(child.payload()).map(Self::Wide)
            } else {
                NarrowSequenceNumberBox::decode_payload(child.payload()).map(Self::Narrow)
            }
        }
    }

    impl SequenceNumber {
        fn encoded_len(&self) -> u64 {
            match self {
                Self::Wide(wide) => wide.encoded_len(),
                Self::Narrow(narrow) => narrow.encoded_len(),
            }
        }
    }

    /// Container of a vendor's own, named by a UUID: exactly one level, one sequence number in either variant, and whatever else
    #[derive(PartialEq, Debug)]
    struct TrackInformationBox {
        level: LevelBox,
        sequence_number: SequenceNumber,
        other_boxes: OtherBoxes,
    }

    impl BoxDefinition for TrackInformationBox {
        const BOX_TYPE: BoxType = BoxType::extended([0x5a; 16]);
    }

    impl BoxDecode for TrackInformationBox {
        fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
            let mut children = ChildBoxes::decode(reader.take_remainder())?;

            Ok(Self {
                level: children.take_exactly_one()?,
                sequence_number: children.take_exactly_one_variant()?,
                other_boxes: OtherBoxes::from(children),
            })
        }
    }

    impl BoxEncode for TrackInformationBox {
        fn payload_len(&self) -> u64 {
            self.other_boxes
                .as_slice()
                .iter()
                .map(AnyBox::encoded_len)
                .chain([self.level.encoded_len(), self.sequence_number.encoded_len()])
                .fold(0, u64::saturating_add)
        }

        fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
            let mut children = ChildBoxWriter::new(writer.take_remainder(), &self.other_boxes);
            children.write(&self.level)?;
            match &self.sequence_number {
                SequenceNumber::Wide(wide) => children.write(wide)?,
                SequenceNumber::Narrow(narrow) => children.write(narrow)?,
            }
            children.finish()
        }
    }

    /// `TrackInformationBox` holding, in this order, its `vlvl`, an unknown `uuid` child, and its `vsq2`
    const READ: &[u8] =
        b"\0\0\0\x46uuid\x5a\x5a\x5a\x5a\x5a\x5a\x5a\x5a\x5a\x5a\x5a\x5a\x5a\x5a\x5a\x5a\
        \0\0\0\x0avlvl\x01\x02\
        \0\0\0\x1auuid\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10\xaa\xbb\
        \0\0\0\x0avsq2\0\x07";

    /// The child of `READ` the container has no field for
    const UNKNOWN_CHILD: &[u8] =
        b"\0\0\0\x1auuid\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10\xaa\xbb";

    #[test]
    fn a_vendor_container_reads_its_children_and_keeps_the_one_it_has_no_field_for() {
        assert_eq!(
            TrackInformationBox::decode(READ),
            Ok((
                TrackInformationBox {
                    level: LevelBox { level: 0x0102 },
                    sequence_number: SequenceNumber::Narrow(NarrowSequenceNumberBox {
                        sequence_number: 7
                    }),
                    other_boxes: OtherBoxes::from(ChildBoxes::decode(UNKNOWN_CHILD).unwrap()),
                },
                b"".as_slice()
            ))
        );
    }

    #[test]
    fn a_vendor_container_writes_the_child_it_has_no_field_for_back_where_it_lay() {
        let (track_information, _) = TrackInformationBox::decode(READ).unwrap();
        let mut written = vec![0; usize::try_from(track_information.encoded_len()).unwrap()];

        track_information.encode(&mut written).unwrap();

        assert_eq!(written, READ);
    }

    #[test]
    fn a_child_that_does_not_read_names_itself_on_the_path_of_the_failure() {
        let truncated_level =
            b"\0\0\0\x2buuid\x5a\x5a\x5a\x5a\x5a\x5a\x5a\x5a\x5a\x5a\x5a\x5a\x5a\x5a\x5a\x5a\
            \0\0\0\x0avsq2\0\x07\
            \0\0\0\x09vlvl\x01";

        assert_eq!(
            TrackInformationBox::decode(truncated_level).map_err(|error| error.to_string()),
            Err("in vlvl: box payload of 2 bytes cut short by an input of 1".to_owned())
        );
    }
}
