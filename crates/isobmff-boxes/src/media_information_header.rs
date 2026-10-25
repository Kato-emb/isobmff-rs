//! [`MediaInformationHeader`], the specific media header of a track, and the boxes it is one of

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, BoxVariants, Error, FieldReader, FieldWriter,
    I8F8, RawBox,
};

use crate::full_box::{read_version_zero, write_version_zero};

/// [`VideoMediaHeaderBox`] (`vmhd`), ISO/IEC 14496-12 §12.1.2
///
/// General presentation information of video media, independent of the
/// coding.
///
/// The `flags` the specification fixes at 1 are read past whatever they hold,
/// and written as 1.
///
/// [`Default`] is the box with every template field at its template value.
#[doc(alias = "vmhd")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct VideoMediaHeaderBox {
    /// Composition mode of the track, 0 being copy over the existing image
    pub graphics_mode: u16,
    /// Red, green and blue colour values available for use by graphics modes
    pub op_color: [u16; 3],
}

impl BoxDefinition for VideoMediaHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"vmhd");
}

impl BoxDecode for VideoMediaHeaderBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {
            graphics_mode: reader.read_u16()?,
            op_color: [reader.read_u16()?, reader.read_u16()?, reader.read_u16()?],
        })
    }
}

impl BoxEncode for VideoMediaHeaderBox {
    fn payload_len(&self) -> u64 {
        4 + 2 + 2 * 3
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&[0, 0, 0, 1])?;
        writer.write_u16(self.graphics_mode)?;
        self.op_color
            .iter()
            .try_for_each(|value| writer.write_u16(*value))
    }
}

/// [`SoundMediaHeaderBox`] (`smhd`), ISO/IEC 14496-12 §12.2.2
///
/// General presentation information of audio media, independent of the
/// coding.
///
/// [`Default`] is the box with every template field at its template value.
#[doc(alias = "smhd")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SoundMediaHeaderBox {
    /// Place of mono audio in a stereo space: 0 is centre, -1.0 full left and 1.0 full right
    pub balance: I8F8,
}

impl Default for SoundMediaHeaderBox {
    fn default() -> Self {
        Self {
            balance: I8F8::ZERO,
        }
    }
}

impl BoxDefinition for SoundMediaHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"smhd");
}

impl BoxDecode for SoundMediaHeaderBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;
        let balance = I8F8::from_raw(reader.read_i16()?);
        reader.read_bytes::<2>()?;

        Ok(Self { balance })
    }
}

impl BoxEncode for SoundMediaHeaderBox {
    fn payload_len(&self) -> u64 {
        4 + 2 + 2
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        writer.write_i16(self.balance.raw())?;
        writer.write_bytes(&[0; 2])
    }
}

/// [`HintMediaHeaderBox`] (`hmhd`), ISO/IEC 14496-12 §12.4.3
///
/// General information of a hint track, independent of the protocol. A PDU
/// is a protocol data unit.
///
/// The field the Syntax spells `sreserved`, fixed at 0, is the `reserved`
/// field every other box spells that way: it is read past whatever it holds,
/// and written as 0.
#[doc(alias = "hmhd")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct HintMediaHeaderBox {
    /// Size in bytes of the largest PDU in the hint stream
    pub max_pdu_size: u16,
    /// Average size of a PDU over the entire presentation
    pub avg_pdu_size: u16,
    /// Maximum rate in bits per second over any window of one second
    pub max_bitrate: u32,
    /// Average rate in bits per second over the entire presentation
    pub avg_bitrate: u32,
}

impl BoxDefinition for HintMediaHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"hmhd");
}

impl BoxDecode for HintMediaHeaderBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;
        let max_pdu_size = reader.read_u16()?;
        let avg_pdu_size = reader.read_u16()?;
        let max_bitrate = reader.read_u32()?;
        let avg_bitrate = reader.read_u32()?;
        reader.read_bytes::<4>()?;

        Ok(Self {
            max_pdu_size,
            avg_pdu_size,
            max_bitrate,
            avg_bitrate,
        })
    }
}

impl BoxEncode for HintMediaHeaderBox {
    fn payload_len(&self) -> u64 {
        4 + 2 + 2 + 4 + 4 + 4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        writer.write_u16(self.max_pdu_size)?;
        writer.write_u16(self.avg_pdu_size)?;
        writer.write_u32(self.max_bitrate)?;
        writer.write_u32(self.avg_bitrate)?;
        writer.write_bytes(&[0; 4])
    }
}

/// [`SubtitleMediaHeaderBox`] (`sthd`), ISO/IEC 14496-12 §12.6.2
///
/// Media header of a subtitle track, which carries no field.
#[doc(alias = "sthd")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct SubtitleMediaHeaderBox {}

impl BoxDefinition for SubtitleMediaHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"sthd");
}

impl BoxDecode for SubtitleMediaHeaderBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {})
    }
}

impl BoxEncode for SubtitleMediaHeaderBox {
    fn payload_len(&self) -> u64 {
        4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)
    }
}

/// [`NullMediaHeaderBox`] (`nmhd`), ISO/IEC 14496-12 §8.4.5.2
///
/// Media header of a stream for which no specific media header is
/// identified, which carries no field.
///
/// The specification assigns none of the `flags` a meaning: they are read
/// past whatever they hold, and written as zero.
#[doc(alias = "nmhd")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct NullMediaHeaderBox {}

impl BoxDefinition for NullMediaHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"nmhd");
}

impl BoxDecode for NullMediaHeaderBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {})
    }
}

impl BoxEncode for NullMediaHeaderBox {
    fn payload_len(&self) -> u64 {
        4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)
    }
}

/// [`VolumetricVisualMediaHeaderBox`] (`vvhd`), ISO/IEC 14496-12 §12.9.2
///
/// Media header of a volumetric visual track, which carries no field.
#[doc(alias = "vvhd")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct VolumetricVisualMediaHeaderBox {}

impl BoxDefinition for VolumetricVisualMediaHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"vvhd");
}

impl BoxDecode for VolumetricVisualMediaHeaderBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {})
    }
}

impl BoxEncode for VolumetricVisualMediaHeaderBox {
    fn payload_len(&self) -> u64 {
        4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)
    }
}

/// Specific media header of a track, ISO/IEC 14496-12 §8.4.5
///
/// A `MediaInformationBox` holds exactly one specific media header, the one
/// that matches the handler of its track. Each variant is one of the media
/// headers ISO/IEC 14496-12 defines; a derived specification may define
/// others.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum MediaInformationHeader {
    /// Media header of a video track
    Video(VideoMediaHeaderBox),
    /// Media header of an audio track
    Sound(SoundMediaHeaderBox),
    /// Media header of a hint track
    Hint(HintMediaHeaderBox),
    /// Media header of a subtitle track
    Subtitle(SubtitleMediaHeaderBox),
    /// Media header of a stream for which no specific media header is identified
    Null(NullMediaHeaderBox),
    /// Media header of a volumetric visual track
    VolumetricVisual(VolumetricVisualMediaHeaderBox),
}

impl BoxVariants for MediaInformationHeader {
    const VARIANTS: &'static [BoxType] = &[
        VideoMediaHeaderBox::BOX_TYPE,
        SoundMediaHeaderBox::BOX_TYPE,
        HintMediaHeaderBox::BOX_TYPE,
        SubtitleMediaHeaderBox::BOX_TYPE,
        NullMediaHeaderBox::BOX_TYPE,
        VolumetricVisualMediaHeaderBox::BOX_TYPE,
    ];

    fn decode_variant(child: RawBox<'_>) -> Result<Self, Error> {
        let box_type = child.header().box_type();
        let payload = child.payload();
        if box_type == VideoMediaHeaderBox::BOX_TYPE {
            VideoMediaHeaderBox::decode_payload(payload).map(Self::Video)
        } else if box_type == SoundMediaHeaderBox::BOX_TYPE {
            SoundMediaHeaderBox::decode_payload(payload).map(Self::Sound)
        } else if box_type == HintMediaHeaderBox::BOX_TYPE {
            HintMediaHeaderBox::decode_payload(payload).map(Self::Hint)
        } else if box_type == SubtitleMediaHeaderBox::BOX_TYPE {
            SubtitleMediaHeaderBox::decode_payload(payload).map(Self::Subtitle)
        } else if box_type == NullMediaHeaderBox::BOX_TYPE {
            NullMediaHeaderBox::decode_payload(payload).map(Self::Null)
        } else {
            VolumetricVisualMediaHeaderBox::decode_payload(payload).map(Self::VolumetricVisual)
        }
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, ChildBoxes, Error, I8F8};

    use super::{
        HintMediaHeaderBox, MediaInformationHeader, NullMediaHeaderBox, SoundMediaHeaderBox,
        SubtitleMediaHeaderBox, VideoMediaHeaderBox, VolumetricVisualMediaHeaderBox,
    };
    use crate::encoded;

    /// `vmhd` of graphics mode 0x40 and the colour (1, 2, 3)
    const VMHD: &[u8] = b"\0\0\0\x14vmhd\0\0\0\x01\0\x40\0\x01\0\x02\0\x03";

    /// `smhd` of balance -0.5
    const SMHD: &[u8] = b"\0\0\0\x10smhd\0\0\0\0\xff\x80\0\0";

    /// `hmhd` of PDUs of at most 1500 bytes and 1000 on average, at 64000 bits per second at most and 32000 on average
    const HMHD: &[u8] = b"\0\0\0\x1chmhd\0\0\0\0\x05\xdc\x03\xe8\0\0\xfa\0\0\0\x7d\0\0\0\0\0";

    /// `sthd`, which carries no field
    const STHD: &[u8] = b"\0\0\0\x0csthd\0\0\0\0";

    /// `nmhd`, which carries no field
    const NMHD: &[u8] = b"\0\0\0\x0cnmhd\0\0\0\0";

    /// `vvhd`, which carries no field
    const VVHD: &[u8] = b"\0\0\0\x0cvvhd\0\0\0\0";

    fn video_media_header() -> VideoMediaHeaderBox {
        VideoMediaHeaderBox {
            graphics_mode: 0x40,
            op_color: [1, 2, 3],
        }
    }

    fn sound_media_header() -> SoundMediaHeaderBox {
        SoundMediaHeaderBox {
            balance: I8F8::from_raw(-0x80),
        }
    }

    fn hint_media_header() -> HintMediaHeaderBox {
        HintMediaHeaderBox {
            max_pdu_size: 1500,
            avg_pdu_size: 1000,
            max_bitrate: 64000,
            avg_bitrate: 32000,
        }
    }

    #[test]
    fn each_media_header_reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            (
                VideoMediaHeaderBox::decode(VMHD),
                SoundMediaHeaderBox::decode(SMHD),
                HintMediaHeaderBox::decode(HMHD),
                SubtitleMediaHeaderBox::decode(STHD),
                NullMediaHeaderBox::decode(NMHD),
                VolumetricVisualMediaHeaderBox::decode(VVHD),
            ),
            (
                Ok((video_media_header(), b"".as_slice())),
                Ok((sound_media_header(), b"".as_slice())),
                Ok((hint_media_header(), b"".as_slice())),
                Ok((SubtitleMediaHeaderBox {}, b"".as_slice())),
                Ok((NullMediaHeaderBox {}, b"".as_slice())),
                Ok((VolumetricVisualMediaHeaderBox {}, b"".as_slice())),
            )
        );
    }

    #[test]
    fn each_media_header_writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            [
                encoded(&video_media_header()),
                encoded(&sound_media_header()),
                encoded(&hint_media_header()),
                encoded(&SubtitleMediaHeaderBox {}),
                encoded(&NullMediaHeaderBox {}),
                encoded(&VolumetricVisualMediaHeaderBox {}),
            ],
            [VMHD, SMHD, HMHD, STHD, NMHD, VVHD].map(<[u8]>::to_vec)
        );
    }

    #[test]
    fn a_child_of_any_variant_reads_as_the_media_header_its_type_names() {
        let read = [VMHD, SMHD, HMHD, STHD, NMHD, VVHD].map(|child| {
            ChildBoxes::decode(child)
                .unwrap()
                .take_exactly_one_variant::<MediaInformationHeader>()
        });

        assert_eq!(
            read,
            [
                Ok(MediaInformationHeader::Video(video_media_header())),
                Ok(MediaInformationHeader::Sound(sound_media_header())),
                Ok(MediaInformationHeader::Hint(hint_media_header())),
                Ok(MediaInformationHeader::Subtitle(SubtitleMediaHeaderBox {})),
                Ok(MediaInformationHeader::Null(NullMediaHeaderBox {})),
                Ok(MediaInformationHeader::VolumetricVisual(
                    VolumetricVisualMediaHeaderBox {}
                )),
            ]
        );
    }

    #[test]
    fn the_flags_of_vmhd_are_read_past_and_written_as_1() {
        let (read, _) =
            VideoMediaHeaderBox::decode(b"\0\0\0\x14vmhd\0\0\0\0\0\x40\0\x01\0\x02\0\x03").unwrap();

        assert_eq!(encoded(&read), VMHD);
    }

    #[test]
    fn reserved_fields_and_the_flags_of_nmhd_are_read_past_and_written_as_zero() {
        let (sound, _) =
            SoundMediaHeaderBox::decode(b"\0\0\0\x10smhd\0\0\0\0\xff\x80\x12\x34").unwrap();
        let (hint, _) = HintMediaHeaderBox::decode(
            b"\0\0\0\x1chmhd\0\0\0\0\x05\xdc\x03\xe8\0\0\xfa\0\0\0\x7d\0\x12\x34\x56\x78",
        )
        .unwrap();
        let (null, _) = NullMediaHeaderBox::decode(b"\0\0\0\x0cnmhd\0\0\0\x05").unwrap();

        assert_eq!(
            [encoded(&sound), encoded(&hint), encoded(&null)],
            [SMHD, HMHD, NMHD].map(<[u8]>::to_vec)
        );
    }

    #[test]
    fn a_payload_ending_inside_a_field_is_refused() {
        assert_eq!(
            (
                VideoMediaHeaderBox::decode_payload(b"\0\0\0\x01\0\x40\0\x01\0"),
                SoundMediaHeaderBox::decode_payload(b"\0\0\0\0\xff"),
                HintMediaHeaderBox::decode_payload(
                    b"\0\0\0\0\x05\xdc\x03\xe8\0\0\xfa\0\0\0\x7d\0\0\0"
                ),
                NullMediaHeaderBox::decode_payload(b"\0\0"),
            ),
            (
                Err(Error::truncated_payload(10, 9)),
                Err(Error::truncated_payload(6, 5)),
                Err(Error::truncated_payload(20, 18)),
                Err(Error::truncated_payload(4, 2)),
            )
        );
    }

    #[test]
    fn a_version_the_boxes_do_not_read_is_refused() {
        let version_1 = b"\x01\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";

        assert_eq!(
            (
                VideoMediaHeaderBox::decode_payload(version_1),
                SoundMediaHeaderBox::decode_payload(version_1),
                HintMediaHeaderBox::decode_payload(version_1),
                SubtitleMediaHeaderBox::decode_payload(version_1),
                NullMediaHeaderBox::decode_payload(version_1),
                VolumetricVisualMediaHeaderBox::decode_payload(version_1),
            ),
            (
                Err(Error::unsupported_version(1)),
                Err(Error::unsupported_version(1)),
                Err(Error::unsupported_version(1)),
                Err(Error::unsupported_version(1)),
                Err(Error::unsupported_version(1)),
                Err(Error::unsupported_version(1)),
            )
        );
    }
}
