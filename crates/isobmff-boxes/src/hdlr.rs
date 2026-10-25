//! [`HandlerBox`], the media type of a track or the format of a meta box

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FourCc,
    Utf8String,
};

use crate::full_box::{read_version_zero, write_version_zero};

/// [`HandlerBox`] (`hdlr`), ISO/IEC 14496-12 §8.4.3
///
/// In a `MediaBox`, declares the media type of the track, and so the process
/// by which its media data is presented; in a `MetaBox`, declares the
/// structure or format of what the `MetaBox` holds.
///
/// [`Default`] is the box with `handler_type` four zero bytes and an empty
/// `name`.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::HandlerBox;
/// use isobmff_core::{BoxDecode, FourCc};
///
/// // The handler of a video track
/// let bytes = b"\0\0\0\x2dhdlr\0\0\0\0\0\0\0\0vide\0\0\0\0\0\0\0\0\0\0\0\0VideoHandler\0";
/// let (handler, _) = HandlerBox::decode(bytes).unwrap();
/// assert_eq!(handler.handler_type, FourCc::new(*b"vide"));
/// assert_eq!(handler.name.as_str(), Some("VideoHandler"));
/// ```
#[doc(alias = "hdlr")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct HandlerBox {
    /// Media type of the track, or the format of what the `MetaBox` holds
    pub handler_type: FourCc,
    /// Human-readable name of the track type, for debugging and inspection
    pub name: Utf8String,
}

impl Default for HandlerBox {
    fn default() -> Self {
        Self {
            handler_type: FourCc::new([0; 4]),
            name: Utf8String::default(),
        }
    }
}

impl BoxDefinition for HandlerBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"hdlr");
}

impl BoxDecode for HandlerBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;
        reader.read_bytes::<4>()?;
        let handler_type = FourCc::new(*reader.read_bytes::<4>()?);
        reader.read_bytes::<{ 4 * 3 }>()?;

        Ok(Self {
            handler_type,
            name: Utf8String::from_slice(reader.take_remainder()),
        })
    }
}

impl BoxEncode for HandlerBox {
    fn payload_len(&self) -> u64 {
        self.name.encoded_len().saturating_add(4 + 4 + 4 + 4 * 3)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        writer.write_bytes(&[0; 4])?;
        writer.write_bytes(self.handler_type.as_bytes())?;
        writer.write_bytes(&[0; 4 * 3])?;
        self.name.encode(writer.take_remainder())?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use isobmff_core::{BoxDecode, Error, FourCc, Utf8String};

    use super::HandlerBox;
    use crate::encoded;

    /// `hdlr` of a sound track named `SoundHandler`
    const HDLR: &[u8] = b"\0\0\0\x2dhdlr\0\0\0\0\0\0\0\0soun\0\0\0\0\0\0\0\0\0\0\0\0SoundHandler\0";

    fn handler() -> HandlerBox {
        HandlerBox {
            handler_type: FourCc::new(*b"soun"),
            name: Utf8String::new(String::from("SoundHandler")).unwrap(),
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(HandlerBox::decode(HDLR), Ok((handler(), b"".as_slice())));
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&handler()), HDLR);
    }

    #[test]
    fn pre_defined_and_reserved_are_read_past_and_written_as_zero() {
        let (read, _) =
            HandlerBox::decode(b"\0\0\0\x2dhdlr\0\0\0\0mhlrsounappl\0\0\0\0\0\0\0\0SoundHandler\0")
                .unwrap();

        assert_eq!(encoded(&read), HDLR);
    }

    #[test]
    fn a_payload_ending_inside_handler_type_is_refused() {
        assert_eq!(
            HandlerBox::decode_payload(b"\0\0\0\0\0\0\0\0so"),
            Err(Error::truncated_payload(12, 10))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            HandlerBox::decode_payload(b"\x01\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
