//! [`HandlerBox`] (`hdlr`), ISO/IEC 14496-12 §8.4.3

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, DecodeError, EncodeError, FourCC, FullBoxFields,
    FullBoxFlags, NullTerminatedString,
};

use crate::field::{split_field, split_field_mut};

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
    /// * [`TruncatedPayload`](DecodeError::TruncatedPayload): the payload ends
    ///   before the fields that precede the `name`.
    /// * [`InvalidUtf8`](DecodeError::InvalidUtf8): the `name` is not UTF-8.
    fn decode_payload(payload: &[u8]) -> Result<Self, DecodeError> {
        let available = u64::try_from(payload.len()).unwrap_or(u64::MAX);
        let (full_box_field, rest) = split_field::<4>(payload, 4, available)?;

        let version = FullBoxFields::from_bytes(full_box_field).version();
        if version != 0 {
            return Err(DecodeError::UnsupportedVersion(version));
        }

        let (pre_defined, rest) = split_field::<4>(rest, FIXED_FIELDS_LEN, available)?;
        let (handler_type, rest) = split_field::<4>(rest, FIXED_FIELDS_LEN, available)?;
        let (_reserved, name) = split_field::<12>(rest, FIXED_FIELDS_LEN, available)?;

        Ok(Self {
            pre_defined: u32::from_be_bytes(*pre_defined),
            handler_type: FourCC::new(*handler_type),
            name: NullTerminatedString::from_slice(name)?,
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
        let mismatch = EncodeError::BufferLengthMismatch { expected, actual };
        if actual != expected {
            return Err(mismatch);
        }

        let (full_box_field, rest) = split_field_mut::<4>(buffer, mismatch)?;
        *full_box_field = FullBoxFields::new(0, FullBoxFlags::ZERO).to_bytes();

        let (pre_defined, rest) = split_field_mut::<4>(rest, mismatch)?;
        *pre_defined = self.pre_defined.to_be_bytes();
        let (handler_type, rest) = split_field_mut::<4>(rest, mismatch)?;
        *handler_type = *self.handler_type.as_bytes();
        let (reserved, rest) = split_field_mut::<12>(rest, mismatch)?;
        *reserved = [0; 12];
        self.name.encode(rest)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, DecodeError, FourCC, NullTerminatedString};

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
            Err(DecodeError::TruncatedPayload {
                needed: 24,
                available: 23
            })
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
