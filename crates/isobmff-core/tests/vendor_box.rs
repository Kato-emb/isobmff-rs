//! Boxes of a crate of their own, implemented on this crate's public items alone

#[cfg(test)]
mod tests {
    use isobmff_core::{
        BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWidth, FieldWriter,
        FullBoxFlags, FullBoxHeader,
    };

    /// Box of a vendor's own, named by a four-character code, whose payload is one 16-bit level
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

    /// Full box of a vendor's own, named by a UUID, carrying a time in 32 or 64 bits by its version
    #[derive(PartialEq, Eq, Debug)]
    struct CaptureTimeBox {
        version: u8,
        capture_time: u64,
    }

    impl BoxDefinition for CaptureTimeBox {
        const BOX_TYPE: BoxType = BoxType::extended([
            0x8f, 0x1c, 0x4e, 0x02, 0x5b, 0x7d, 0x4a, 0x61, 0x9e, 0x33, 0x0c, 0x5d, 0x21, 0x84,
            0xb7, 0x6a,
        ]);
    }

    impl CaptureTimeBox {
        fn width(version: u8) -> Result<FieldWidth, Error> {
            match version {
                0 => Ok(FieldWidth::Compact),
                1 => Ok(FieldWidth::Extended),
                _ => Err(Error::unsupported_version(version)),
            }
        }
    }

    impl BoxDecode for CaptureTimeBox {
        fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
            let header = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?);
            let width = Self::width(header.version())?;

            Ok(Self {
                version: header.version(),
                capture_time: reader.read_unsigned(width)?,
            })
        }
    }

    impl BoxEncode for CaptureTimeBox {
        fn payload_len(&self) -> u64 {
            if self.version == 0 { 8 } else { 12 }
        }

        fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
            let header = FullBoxHeader::new(self.version, FullBoxFlags::ZERO);
            writer.write_bytes(&header.to_bytes())?;
            writer.write_unsigned(Self::width(self.version)?, self.capture_time)
        }
    }

    /// `CaptureTimeBox` of version 1, as ISO/IEC 14496-12 §4.2.2 lays out a `uuid` full box
    const CAPTURE_TIME_VERSION_1: [u8; 36] = [
        0x00, 0x00, 0x00, 0x24, b'u', b'u', b'i', b'd', 0x8f, 0x1c, 0x4e, 0x02, 0x5b, 0x7d, 0x4a,
        0x61, 0x9e, 0x33, 0x0c, 0x5d, 0x21, 0x84, 0xb7, 0x6a, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x01, 0x00, 0x00, 0x00, 0x2a,
    ];

    #[test]
    fn a_compact_vendor_box_writes_the_bytes_its_syntax_lays_out() {
        let mut buffer = [0; 10];

        LevelBox { level: 0x0102 }.encode(&mut buffer).unwrap();

        assert_eq!(&buffer, b"\0\0\0\x0avlvl\x01\x02");
    }

    #[test]
    fn a_uuid_full_box_writes_the_bytes_its_syntax_lays_out() {
        let capture_time = CaptureTimeBox {
            version: 1,
            capture_time: 0x0000_0001_0000_002a,
        };
        let mut buffer = [0; 36];

        capture_time.encode(&mut buffer).unwrap();

        assert_eq!(buffer, CAPTURE_TIME_VERSION_1);
    }

    #[test]
    fn a_uuid_full_box_reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            CaptureTimeBox::decode(&CAPTURE_TIME_VERSION_1),
            Ok((
                CaptureTimeBox {
                    version: 1,
                    capture_time: 0x0000_0001_0000_002a,
                },
                b"".as_slice()
            ))
        );
    }

    #[test]
    fn syntax_a_later_revision_added_at_the_end_of_a_full_box_is_skipped() {
        let extended = b"\0\0\0\x22uuid\x8f\x1c\x4e\x02\x5b\x7d\x4a\x61\x9e\x33\x0c\x5d\x21\x84\xb7\x6a\0\0\0\0\0\0\0\x2a\xff\xff";

        assert_eq!(
            CaptureTimeBox::decode(extended),
            Ok((
                CaptureTimeBox {
                    version: 0,
                    capture_time: 0x2a,
                },
                b"".as_slice()
            ))
        );
    }

    #[test]
    fn a_full_box_of_a_version_the_vendor_does_not_read_is_left_to_skip() {
        let version_2 = b"\0\0\0\x1cuuid\x8f\x1c\x4e\x02\x5b\x7d\x4a\x61\x9e\x33\x0c\x5d\x21\x84\xb7\x6a\x02\0\0\0";

        assert_eq!(
            CaptureTimeBox::decode(version_2),
            Err(Error::unsupported_version(2))
        );
    }

    #[test]
    fn a_value_too_wide_for_the_version_it_is_written_under_is_refused() {
        let capture_time = CaptureTimeBox {
            version: 0,
            capture_time: 0x0000_0001_0000_0000,
        };

        assert_eq!(
            capture_time.encode(&mut [0; 32]),
            Err(Error::out_of_range(
                0x0000_0001_0000_0000,
                FieldWidth::Compact
            ))
        );
    }
}
