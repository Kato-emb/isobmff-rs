//! [`VideoMediaHeaderBox`] (`vmhd`), ISO/IEC 14496-12 §12.1.2

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFields,
    FullBoxFlags,
};

/// Length of the payload, which has no version-dependent field
const PAYLOAD_LEN: u64 = 12;

/// Colour values the box carries, one per channel
const OP_COLOR_CHANNELS: usize = 3;

/// Value §12.1.2 declares for the `flags` field of this box
const DECLARED_FLAGS: FullBoxFlags = match FullBoxFlags::new(1) {
    Some(flags) => flags,
    // Why not unwrap: 1 is within the 24 bits the field carries, so the flags
    // always build, and a degenerate value stands in for the panic the lints
    // forbid.
    None => FullBoxFlags::ZERO,
};

/// Box that states how the video of a track is composed onto what is under it
///
/// [`VideoMediaHeaderBox`] (`vmhd`), ISO/IEC 14496-12 §12.1.2. A video track
/// takes this as the media header its `minf` must hold. The `graphics_mode`
/// picks a composition mode, of which the spec defines one — `copy`, 0, which
/// puts the image over what is there — and `op_color` is the red, green, and
/// blue a mode that takes colours works from.
///
/// Neither the version nor the `flags` are held: the spec declares the version
/// zero and the flags 1 for this box, and the flags a file states are not read.
#[doc(alias = "vmhd")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct VideoMediaHeaderBox {
    graphics_mode: u16,
    op_color: [u16; OP_COLOR_CHANNELS],
}

impl VideoMediaHeaderBox {
    /// Creates the box from the composition mode and the colours it works from
    #[must_use]
    pub const fn new(graphics_mode: u16, op_color: [u16; OP_COLOR_CHANNELS]) -> Self {
        Self {
            graphics_mode,
            op_color,
        }
    }

    /// Returns the mode the video of this track is composed with
    #[must_use]
    pub const fn graphics_mode(&self) -> u16 {
        self.graphics_mode
    }

    /// Returns the red, green, and blue a composition mode works from
    #[must_use]
    pub const fn op_color(&self) -> [u16; OP_COLOR_CHANNELS] {
        self.op_color
    }
}

impl BoxDefinition for VideoMediaHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"vmhd");
}

impl BoxDecode for VideoMediaHeaderBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
    ///   ends inside a field of the box.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version != 0 {
            return Err(Error::unsupported_version(version));
        }

        let graphics_mode = reader.read_u16()?;
        let mut op_color = [0; OP_COLOR_CHANNELS];
        for channel in &mut op_color {
            *channel = reader.read_u16()?;
        }

        Ok(Self {
            graphics_mode,
            op_color,
        })
    }
}

impl BoxEncode for VideoMediaHeaderBox {
    fn payload_len(&self) -> u64 {
        PAYLOAD_LEN
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxFields::new(0, DECLARED_FLAGS).to_bytes())?;
        writer.write_u16(self.graphics_mode)?;
        for channel in self.op_color {
            writer.write_u16(channel)?;
        }

        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::VideoMediaHeaderBox;

    /// Media header of a video track composing its image over what is under it
    pub(crate) fn video_media_header() -> VideoMediaHeaderBox {
        VideoMediaHeaderBox::new(0, [0; 3])
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(video_media_header: &VideoMediaHeaderBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(video_media_header.payload_len()).unwrap()];
        video_media_header.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let video_media_header = VideoMediaHeaderBox::new(1, [0x1234, 0x5678, 0x9abc]);

        let payload = encoded_payload(&video_media_header);

        assert_eq!(payload, b"\0\0\0\x01\0\x01\x12\x34\x56\x78\x9a\xbc");
        assert_eq!(
            VideoMediaHeaderBox::decode_payload(&payload).unwrap(),
            video_media_header
        );
    }

    #[test]
    fn the_flags_the_spec_declares_are_written_whatever_the_file_stated() {
        let payload = [b"\0\0\0\0".as_slice(), &[0; 8]].concat();

        let video_media_header = VideoMediaHeaderBox::decode_payload(&payload).unwrap();

        assert_eq!(
            encoded_payload(&video_media_header).get(..4),
            Some(b"\0\0\0\x01".as_slice())
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let mut payload = vec![0; 12];
        *payload.first_mut().unwrap() = 1;

        assert_eq!(
            VideoMediaHeaderBox::decode_payload(&payload),
            Err(Error::unsupported_version(1))
        );
    }

    #[test]
    fn a_payload_shorter_than_the_fields_is_rejected() {
        assert_eq!(
            VideoMediaHeaderBox::decode_payload(&[0; 11]),
            Err(Error::truncated_payload(12, 11))
        );
    }
}
