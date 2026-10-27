//! [`StereoVideoBox`], the stereo arrangement of the frames of a restricted video track

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::full_box::{read_version_zero, write_version_zero};

/// [`StereoVideoBox`] (`stvi`), ISO/IEC 14496-12 §13.5.4.2
///
/// Indicates that the decoded frames either pack two constituent frames
/// that form a stereo pair, or hold one of the two views of a stereo pair.
/// The box lies in the `SchemeInformationBox` of a restricted scheme of the
/// `scheme_type` `stvi`.
///
/// `single_view_allowed` takes the 2 bits after the 30 `reserved` bits,
/// which are read past and written as zero; a value wider than 2 bits is
/// refused with [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
/// `single_view_allowed` when the box is written. `length` is the length of
/// [`stereo_indication_type`](Self::stereo_indication_type). The boxes after
/// it are kept in [`other_boxes`](Self::other_boxes), in the order they came,
/// and written back in it.
///
/// A reserved `stereo_scheme`, or a `stereo_indication_type` other than the
/// scheme lays out, reads as found, and is refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) when the box is
/// written: for `stereo_scheme` when the scheme is reserved, for
/// `length` when the type is not 4 bytes long for the schemes 1 and 2, 2
/// bytes for 3 and 4, or 3 bytes for 5, and for `stereo_indication_type`
/// when the reserved bits of its second byte, past its least significant
/// bit, are set for the schemes 3 to 5.
///
/// # Untrusted input
///
/// `stereo_indication_type` is read only once `length` is held against the
/// payload, so what the box allocates is bounded by the length of the
/// payload, which the caller sets.
#[doc(alias = "stvi")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct StereoVideoBox {
    /// Views a monoscopic display may show: 0 none, the bit 1 the right view, the bit 2 the left view
    pub single_view_allowed: u8,
    /// Scheme of the stereo arrangement and of `stereo_indication_type`, such as 1 for the frame packing arrangement SEI message of ISO/IEC 14496-10
    pub stereo_scheme: u32,
    /// Stereo arrangement type, laid out as `stereo_scheme` defines
    pub stereo_indication_type: Vec<u8>,
    /// Boxes after `stereo_indication_type`, `any_box` in the syntax
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for StereoVideoBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"stvi");
}

impl BoxDecode for StereoVideoBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;
        let [.., single_view_allowed] = reader.read_u32()?.to_be_bytes();
        let stereo_scheme = reader.read_u32()?;
        let length = reader.read_u32()?;
        let stereo_indication_type = reader
            .read_slice(usize::try_from(length).unwrap_or(usize::MAX))?
            .to_vec();

        Ok(Self {
            single_view_allowed: single_view_allowed & 0b11,
            stereo_scheme,
            stereo_indication_type,
            other_boxes: OtherBoxes::from(ChildBoxes::decode(reader.take_remainder())?),
        })
    }
}

impl BoxEncode for StereoVideoBox {
    fn payload_len(&self) -> u64 {
        self.other_boxes
            .as_slice()
            .iter()
            .map(AnyBox::encoded_len)
            .fold(
                u64::try_from(self.stereo_indication_type.len()).unwrap_or(u64::MAX),
                u64::saturating_add,
            )
            .saturating_add(4 + 4 + 4 + 4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.single_view_allowed > 0b11 {
            return Err(Error::invalid_value("single_view_allowed"));
        }
        let (length, flags) = match (self.stereo_scheme, self.stereo_indication_type.as_slice()) {
            (1 | 2, [_, _, _, _]) => (4, 0),
            (3 | 4, [_, second]) => (2, *second),
            (5, [_, second, _]) => (3, *second),
            (1..=5, _) => return Err(Error::invalid_value("length")),
            _ => return Err(Error::invalid_value("stereo_scheme")),
        };
        if flags & !1 != 0 {
            return Err(Error::invalid_value("stereo_indication_type"));
        }
        write_version_zero(writer)?;
        writer.write_u32(u32::from(self.single_view_allowed))?;
        writer.write_u32(self.stereo_scheme)?;
        writer.write_u32(length)?;
        writer.write_slice(&self.stereo_indication_type)?;
        let mut rest = writer.take_remainder();
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, ChildBoxes, Error, OtherBoxes};

    use super::StereoVideoBox;
    use crate::encoded;

    /// `stvi` of the frame packing scheme, of the arrangement type 3 (side by side), that no single view may be shown of
    const STVI: &[u8] = b"\0\0\0\x1cstvi\0\0\0\0\0\0\0\0\0\0\0\x01\0\0\0\x04\0\0\0\x03";

    /// Box after the stereo indication type
    const EXTRA: &[u8] = b"\0\0\0\x09xtra\x01";

    fn stereo_video() -> StereoVideoBox {
        StereoVideoBox {
            single_view_allowed: 0,
            stereo_scheme: 1,
            stereo_indication_type: vec![0, 0, 0, 3],
            other_boxes: OtherBoxes::new(),
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            StereoVideoBox::decode(STVI),
            Ok((stereo_video(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&stereo_video()), STVI);
    }

    #[test]
    fn reads_the_views_a_monoscopic_display_may_show_past_the_reserved_bits() {
        let payload = b"\0\0\0\0\xff\xff\xff\xfe\0\0\0\x03\0\0\0\x02\x03\x01";
        let expected = StereoVideoBox {
            single_view_allowed: 2,
            stereo_scheme: 3,
            stereo_indication_type: vec![3, 1],
            other_boxes: OtherBoxes::new(),
        };

        assert_eq!(StereoVideoBox::decode_payload(payload), Ok(expected));
    }

    #[test]
    fn keeps_the_boxes_after_the_stereo_indication_type() {
        let with_box = StereoVideoBox {
            other_boxes: OtherBoxes::from(ChildBoxes::decode(EXTRA).unwrap()),
            ..stereo_video()
        };
        let bytes = [b"\0\0\0\x25stvi".as_slice(), STVI.get(8..).unwrap(), EXTRA].concat();

        assert_eq!(
            StereoVideoBox::decode(&bytes),
            Ok((with_box.clone(), b"".as_slice()))
        );
        assert_eq!(encoded(&with_box), bytes);
    }

    #[test]
    fn a_length_past_the_end_of_the_payload_is_refused() {
        assert_eq!(
            StereoVideoBox::decode_payload(b"\0\0\0\0\0\0\0\0\0\0\0\x01\xff\xff\xff\xff\0\0\0\x03"),
            Err(Error::truncated_payload(16 + 0xffff_ffff, 20))
        );
    }

    #[test]
    fn a_payload_ending_inside_a_field_is_refused() {
        assert_eq!(
            StereoVideoBox::decode_payload(b"\0\0\0\0\0\0\0\0\0\0"),
            Err(Error::truncated_payload(12, 10))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            StereoVideoBox::decode_payload(b"\x01\0\0\0\0\0\0\0\0\0\0\x01\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }

    #[test]
    fn a_reserved_scheme_is_read_as_found() {
        let expected = StereoVideoBox {
            stereo_scheme: 6,
            stereo_indication_type: vec![1],
            ..stereo_video()
        };

        assert_eq!(
            StereoVideoBox::decode_payload(b"\0\0\0\0\0\0\0\0\0\0\0\x06\0\0\0\x01\x01"),
            Ok(expected)
        );
    }

    #[test]
    fn a_reserved_scheme_is_refused_when_written() {
        let reserved = StereoVideoBox {
            stereo_scheme: 6,
            ..stereo_video()
        };

        assert_eq!(
            reserved.encode(&mut [0; 28]).map(|_| ()),
            Err(Error::invalid_value("stereo_scheme"))
        );
    }

    #[test]
    fn an_indication_type_of_another_length_than_the_scheme_lays_out_is_refused_when_written() {
        for (stereo_scheme, stereo_indication_type) in [
            (1, vec![]),
            (2, vec![0, 3]),
            (3, vec![0, 0, 3, 0]),
            (4, vec![3]),
            (5, vec![3, 0]),
        ] {
            let mislaid = StereoVideoBox {
                stereo_scheme,
                stereo_indication_type,
                ..stereo_video()
            };

            assert_eq!(
                mislaid.encode(&mut [0; 28]).map(|_| ()),
                Err(Error::invalid_value("length"))
            );
        }
    }

    #[test]
    fn an_indication_type_with_reserved_bits_set_is_refused_when_written() {
        for (stereo_scheme, stereo_indication_type) in [
            (3, vec![3, 0b10]),
            (4, vec![3, 0x80]),
            (5, vec![3, 0b11, 1]),
        ] {
            let reserved_bits = StereoVideoBox {
                stereo_scheme,
                stereo_indication_type,
                ..stereo_video()
            };

            assert_eq!(
                reserved_bits.encode(&mut [0; 28]).map(|_| ()),
                Err(Error::invalid_value("stereo_indication_type"))
            );
        }
    }

    #[test]
    fn writes_the_two_and_three_byte_layouts_of_the_schemes() {
        let video_frame_packing = StereoVideoBox {
            stereo_scheme: 4,
            stereo_indication_type: vec![3, 1],
            ..stereo_video()
        };
        let packed_content = StereoVideoBox {
            stereo_scheme: 5,
            stereo_indication_type: vec![3, 1, 1],
            ..stereo_video()
        };

        assert_eq!(
            encoded(&video_frame_packing),
            b"\0\0\0\x1astvi\0\0\0\0\0\0\0\0\0\0\0\x04\0\0\0\x02\x03\x01"
        );
        assert_eq!(
            encoded(&packed_content),
            b"\0\0\0\x1bstvi\0\0\0\0\0\0\0\0\0\0\0\x05\0\0\0\x03\x03\x01\x01"
        );
    }

    #[test]
    fn views_wider_than_two_bits_are_refused() {
        let wide = StereoVideoBox {
            single_view_allowed: 4,
            ..stereo_video()
        };

        assert_eq!(
            wide.encode(&mut [0; 28]).map(|_| ()),
            Err(Error::invalid_value("single_view_allowed"))
        );
    }
}
