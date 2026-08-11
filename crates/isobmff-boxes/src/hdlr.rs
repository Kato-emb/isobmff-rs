//! [`HandlerBox`] (`hdlr`), ISO/IEC 14496-12 §8.4.3

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, DecodeError, EncodeError, FieldReader,
    FieldWriter, FourCC, FullBoxFields, FullBoxFlags, NullTerminatedString,
};

/// Length of the fields that precede the `name`
const FIXED_FIELDS_LEN: u64 = 24;

/// Box that names the handler which presents the media of a track
///
/// [`HandlerBox`] (`hdlr`), ISO/IEC 14496-12 §8.4.3. The `handler_type` is what
/// tells a reader which kind of media a track carries — `vide` for video,
/// `soun` for audio — and the `name` is human-readable text a tool may show.
///
/// The `name` is read leniently: files that leave its terminator off are common,
/// and [`NullTerminatedString`] accepts them. Writing puts a terminator back.
#[doc(alias = "hdlr")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct HandlerBox {
    pre_defined: u32,
    handler_type: FourCC,
    name: NullTerminatedString,
}

impl HandlerBox {
    /// Creates the box from the handler it names
    ///
    /// The `pre_defined` field is left zero, as the spec declares it for a file
    /// written to this revision.
    #[must_use]
    pub const fn new(handler_type: FourCC, name: NullTerminatedString) -> Self {
        Self {
            pre_defined: 0,
            handler_type,
            name,
        }
    }

    /// Returns the field the spec reserves for a later definition
    ///
    /// The value is carried through un-inspected: this box reads no meaning
    /// into it and does not zero it on the way out, so a file that puts data
    /// here — as writers in the QuickTime line do — reads back as the bytes it
    /// was written with.
    #[must_use]
    pub const fn pre_defined(&self) -> u32 {
        self.pre_defined
    }

    /// Returns the code naming the kind of media the track carries
    #[must_use]
    pub const fn handler_type(&self) -> FourCC {
        self.handler_type
    }

    /// Returns the human-readable name of the handler
    #[must_use]
    pub const fn name(&self) -> &NullTerminatedString {
        &self.name
    }
}

impl BoxDefinition for HandlerBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"hdlr");
}

impl BoxDecode for HandlerBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](DecodeError::UnsupportedVersion): the box
    ///   declares a version other than 0.
    /// * [`Field`](DecodeError::Field): the payload ends before the fields that
    ///   precede the `name`.
    /// * [`InvalidUtf8`](DecodeError::InvalidUtf8): the `name` is not UTF-8.
    fn decode_payload(payload: &[u8]) -> Result<Self, DecodeError> {
        let mut reader = FieldReader::new(payload);
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version != 0 {
            return Err(DecodeError::UnsupportedVersion(version));
        }

        let pre_defined = reader.read_u32()?;
        let handler_type = FourCC::new(*reader.read_bytes::<4>()?);
        let _reserved = reader.read_bytes::<12>()?;

        Ok(Self {
            pre_defined,
            handler_type,
            name: NullTerminatedString::from_slice(reader.remainder())?,
        })
    }
}

impl BoxEncode for HandlerBox {
    fn payload_len(&self) -> u64 {
        FIXED_FIELDS_LEN.saturating_add(self.name.encoded_len())
    }

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), EncodeError> {
        let expected = self.payload_len();
        let actual = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        if actual != expected {
            return Err(EncodeError::BufferLengthMismatch { expected, actual });
        }

        let mut writer = FieldWriter::new(buffer);
        writer.write_bytes(&FullBoxFields::new(0, FullBoxFlags::ZERO).to_bytes())?;
        writer.write_u32(self.pre_defined)?;
        writer.write_bytes(self.handler_type.as_bytes())?;
        writer.write_bytes(&[0; 12])?;
        self.name.encode(writer.into_remainder())?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec;

    use isobmff_core::{
        BoxDecode, BoxEncode, DecodeError, FieldReadError, FourCC, NullTerminatedString,
    };

    use super::HandlerBox;

    /// Handler box for a video track, named as most writers name it
    fn video_handler() -> HandlerBox {
        HandlerBox::new(
            FourCC::new(*b"vide"),
            NullTerminatedString::new(String::from("VideoHandler")).unwrap(),
        )
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let handler = video_handler();
        let mut payload = vec![0; usize::try_from(handler.payload_len()).unwrap()];

        handler.encode_payload(&mut payload).unwrap();

        assert_eq!(HandlerBox::decode_payload(&payload).unwrap(), handler);
    }

    #[test]
    fn a_name_left_unterminated_reads_as_the_same_box() {
        let terminated = video_handler();
        let mut payload = vec![0; usize::try_from(terminated.payload_len()).unwrap()];
        terminated.encode_payload(&mut payload).unwrap();
        payload.pop();

        assert_eq!(HandlerBox::decode_payload(&payload).unwrap(), terminated);
    }

    #[test]
    fn a_box_holding_no_name_reads_as_the_empty_string() {
        let handler = HandlerBox::decode_payload(&[0; 24]).unwrap();

        assert_eq!(handler.name().as_str(), "");
        assert_eq!(handler.payload_len(), 25);
    }

    #[test]
    fn a_payload_ending_before_the_name_is_rejected_as_truncated() {
        assert!(matches!(
            HandlerBox::decode_payload(&[0; 23]),
            Err(DecodeError::Field(FieldReadError::UnexpectedEof {
                needed: 24,
                available: 23
            }))
        ));
    }

    #[test]
    fn a_name_that_is_not_utf8_is_rejected() {
        let mut payload = vec![0; 24];
        payload.push(0xff);

        assert!(matches!(
            HandlerBox::decode_payload(&payload),
            Err(DecodeError::InvalidUtf8(_))
        ));
    }
}
