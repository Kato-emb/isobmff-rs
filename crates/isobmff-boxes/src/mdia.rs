//! [`MediaBox`] (`mdia`), ISO/IEC 14496-12 §8.4.1

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, BoxWrite as _, ChildBoxes, Error,
    FieldReader, FieldWriter, OtherBoxes, boxes,
};

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
    other_boxes: OtherBoxes,
}

impl MediaBox {
    /// Creates the box from the three declarations the spec requires
    #[must_use]
    pub const fn new(mdhd: MediaHeaderBox, hdlr: HandlerBox, minf: MediaInformationBox) -> Self {
        Self {
            mdhd,
            hdlr,
            minf,
            other_boxes: OtherBoxes::new(),
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
        self.other_boxes.as_slice()
    }
}

impl BoxDefinition for MediaBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mdia");
}

impl BoxDecode for MediaBox {
    /// # Errors
    ///
    /// * The failures of [`boxes`]: a child does not frame as a box.
    /// * [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox): no `mdhd`,
    ///   `hdlr`, or `minf`.
    /// * [`DuplicateBox`](isobmff_core::ErrorKind::DuplicateBox): more than one of any of
    ///   them.
    /// * Whatever the child reports, on the [`containers`](Error::containers) path: one of them
    ///   does not decode.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut mdhd_boxes = ChildBoxes::new();
        let mut hdlr_boxes = ChildBoxes::new();
        let mut minf_boxes = ChildBoxes::new();
        let mut other_boxes = OtherBoxes::new();

        for child in boxes(reader.take_remainder()) {
            let child = child?;
            let box_type = child.header().box_type();

            if box_type == MediaHeaderBox::BOX_TYPE {
                mdhd_boxes.push(child);
            } else if box_type == HandlerBox::BOX_TYPE {
                hdlr_boxes.push(child);
            } else if box_type == MediaInformationBox::BOX_TYPE {
                minf_boxes.push(child);
            } else {
                other_boxes.keep(child);
            }
        }

        Ok(Self {
            mdhd: mdhd_boxes.exactly_one()?,
            hdlr: hdlr_boxes.exactly_one()?,
            minf: minf_boxes.exactly_one()?,
            other_boxes,
        })
    }
}

impl BoxEncode for MediaBox {
    fn payload_len(&self) -> u64 {
        let others = self
            .other_boxes
            .as_slice()
            .iter()
            .fold(0_u64, |total, other| {
                total.saturating_add(other.encoded_len())
            });

        self.mdhd
            .encoded_len()
            .saturating_add(self.hdlr.encoded_len())
            .saturating_add(self.minf.encoded_len())
            .saturating_add(others)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = self.mdhd.encode(writer.take_remainder())?;
        rest = self.hdlr.encode(rest)?;
        rest = self.minf.encode(rest)?;
        for other in self.other_boxes.as_slice() {
            rest = other.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{
        BoxDecode, BoxEncode, BoxType, Error, FourCC, LanguageCode, NullTerminatedString,
        QuickTimeDateTime,
    };

    use super::MediaBox;
    use crate::hdlr::HandlerBox;
    use crate::mdhd::MediaHeaderBox;
    use crate::minf::MediaInformationBox;
    use crate::stbl::SampleTableBox;
    use crate::stsd::SampleDescriptionBox;

    /// Media box of a video track, with every mandatory child in place
    fn media() -> MediaBox {
        MediaBox::new(
            MediaHeaderBox::new(
                QuickTimeDateTime::from_seconds(0),
                QuickTimeDateTime::from_seconds(0),
                90_000,
                90_000,
                LanguageCode::UND,
            ),
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

        assert_eq!(
            MediaBox::decode_payload(&payload),
            Err(Error::missing_mandatory_box(BoxType::compact(*b"hdlr")))
        );
    }

    #[test]
    fn a_failure_inside_a_child_names_the_path_down_to_it() {
        let mut payload = encoded_payload(&media());
        *payload.get_mut(8).unwrap() = 2;

        let error = MediaBox::decode_payload(&payload).unwrap_err();

        assert_eq!(
            error,
            Error::unsupported_version(2).in_container(BoxType::compact(*b"mdhd"))
        );
    }
}
