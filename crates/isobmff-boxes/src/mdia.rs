//! [`MediaBox`] (`mdia`), ISO/IEC 14496-12 §8.4.1

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, BoxWrite as _, DecodeError, EncodeError,
    boxes,
};

use crate::container::{keep_unpromoted, promote_once, require, total_encoded_len, write_all};
use crate::hdlr::HandlerBox;
use crate::mdhd::MediaHeaderBox;
use crate::minf::MediaInformationBox;

/// Box that holds everything declaring the media of one track
///
/// [`MediaBox`] (`mdia`), ISO/IEC 14496-12 §8.4.1. All three children the spec
/// marks mandatory are promoted to fields, so a `mdia` that decodes has its
/// media header, its handler, and its media information.
///
/// On encode the children are written in the order the spec lists them —
/// `mdhd`, `hdlr`, `minf` — and then the children no field claims, so a
/// round-trip settles the order rather than preserving it.
#[doc(alias = "mdia")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct MediaBox {
    mdhd: MediaHeaderBox,
    hdlr: HandlerBox,
    minf: MediaInformationBox,
    other_boxes: Vec<AnyBox>,
}

impl MediaBox {
    /// Creates the box from the three declarations the spec requires
    #[must_use]
    pub const fn new(mdhd: MediaHeaderBox, hdlr: HandlerBox, minf: MediaInformationBox) -> Self {
        Self {
            mdhd,
            hdlr,
            minf,
            other_boxes: Vec::new(),
        }
    }

    /// Returns the declarations the track's media applies as a whole
    #[must_use]
    pub const fn mdhd(&self) -> &MediaHeaderBox {
        &self.mdhd
    }

    /// Returns the handler naming the kind of media the track carries
    #[must_use]
    pub const fn hdlr(&self) -> &HandlerBox {
        &self.hdlr
    }

    /// Returns the declarations specific to that kind of media
    #[must_use]
    pub const fn minf(&self) -> &MediaInformationBox {
        &self.minf
    }

    /// Returns the children no field of this box claims, in the order they came
    #[must_use]
    pub fn other_boxes(&self) -> &[AnyBox] {
        &self.other_boxes
    }
}

impl BoxDefinition for MediaBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mdia");
}

impl BoxDecode for MediaBox {
    /// # Errors
    ///
    /// * [`Framing`](DecodeError::Framing): a child does not frame as a box.
    /// * [`MissingMandatoryBox`](DecodeError::MissingMandatoryBox): no `mdhd`,
    ///   `hdlr`, or `minf`.
    /// * [`DuplicateBox`](DecodeError::DuplicateBox): more than one of any of
    ///   them.
    /// * [`Child`](DecodeError::Child): one of them does not decode.
    fn decode_payload(payload: &[u8]) -> Result<Self, DecodeError> {
        let mut mdhd = None;
        let mut hdlr = None;
        let mut minf = None;
        let mut other_boxes = Vec::new();

        for child in boxes(payload) {
            let child = child?;
            let box_type = child.header().box_type();

            if box_type == MediaHeaderBox::BOX_TYPE {
                promote_once(&mut mdhd, child)?;
            } else if box_type == HandlerBox::BOX_TYPE {
                promote_once(&mut hdlr, child)?;
            } else if box_type == MediaInformationBox::BOX_TYPE {
                promote_once(&mut minf, child)?;
            } else {
                keep_unpromoted(&mut other_boxes, child);
            }
        }

        Ok(Self {
            mdhd: require(mdhd)?,
            hdlr: require(hdlr)?,
            minf: require(minf)?,
            other_boxes,
        })
    }
}

impl BoxEncode for MediaBox {
    fn payload_len(&self) -> u64 {
        self.mdhd
            .encoded_len()
            .saturating_add(self.hdlr.encoded_len())
            .saturating_add(self.minf.encoded_len())
            .saturating_add(total_encoded_len(&self.other_boxes))
    }

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), EncodeError> {
        let expected = self.payload_len();
        let actual = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        if actual != expected {
            return Err(EncodeError::BufferLengthMismatch { expected, actual });
        }

        let rest = self.mdhd.encode(buffer)?;
        let rest = self.hdlr.encode(rest)?;
        let rest = self.minf.encode(rest)?;
        write_all(&self.other_boxes, rest)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, DecodeError, FourCC, NullTerminatedString};

    use super::MediaBox;
    use crate::hdlr::HandlerBox;
    use crate::mdhd::MediaHeaderBox;
    use crate::minf::MediaInformationBox;
    use crate::stbl::SampleTableBox;
    use crate::stsd::SampleDescriptionBox;

    /// Media box of a video track, with every mandatory child in place
    fn media() -> MediaBox {
        MediaBox::new(
            MediaHeaderBox::new(0, 0, 90_000, 90_000, 0x55C4).unwrap(),
            HandlerBox::new(
                FourCC::new(*b"vide"),
                NullTerminatedString::new(String::from("VideoHandler")).unwrap(),
            ),
            MediaInformationBox::new(SampleTableBox::new(SampleDescriptionBox::new(Vec::new()))),
        )
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(media: &MediaBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(media.payload_len()).unwrap()];
        media.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let payload = encoded_payload(&media());

        assert_eq!(MediaBox::decode_payload(&payload).unwrap(), media());
    }

    #[test]
    fn a_box_missing_the_handler_is_rejected() {
        let whole = encoded_payload(&media());
        let handler_len = usize::try_from(media().hdlr().payload_len()).unwrap() + 8;
        let mdhd_len = usize::try_from(media().mdhd().payload_len()).unwrap() + 8;
        let payload = [
            whole.get(..mdhd_len).unwrap(),
            whole.get(mdhd_len + handler_len..).unwrap(),
        ]
        .concat();

        assert!(matches!(
            MediaBox::decode_payload(&payload),
            Err(DecodeError::MissingMandatoryBox(_))
        ));
    }

    #[test]
    fn a_failure_inside_a_child_names_the_path_down_to_it() {
        let mut payload = encoded_payload(&media());
        *payload.get_mut(8).unwrap() = 2;

        let error = MediaBox::decode_payload(&payload).unwrap_err();

        assert!(matches!(error, DecodeError::Child { .. }));
    }
}
