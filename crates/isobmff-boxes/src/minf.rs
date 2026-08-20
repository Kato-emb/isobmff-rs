//! [`MediaInformationBox`] (`minf`), ISO/IEC 14496-12 §8.4.4

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, BoxWrite as _, ChildBoxes, Error,
    OtherBoxes, boxes,
};

use crate::stbl::SampleTableBox;

/// Box that holds every declaration specific to the media of one track
///
/// [`MediaInformationBox`] (`minf`), ISO/IEC 14496-12 §8.4.4. The `stbl` child
/// is promoted to a field of its own; the media header the track's kind selects
/// — `vmhd`, `smhd`, `hmhd`, or `nmhd` — and the `dinf` that says where the
/// media data lives have no fields yet, so they are kept in
/// [`other_boxes`](Self::other_boxes) and written back unread. The spec marks
/// both mandatory, and decoding does **not** enforce it.
#[doc(alias = "minf")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct MediaInformationBox {
    stbl: SampleTableBox,
    other_boxes: OtherBoxes,
}

impl MediaInformationBox {
    /// Creates the box from the sample table locating the track's samples
    #[must_use]
    pub const fn new(stbl: SampleTableBox) -> Self {
        Self {
            stbl,
            other_boxes: OtherBoxes::new(),
        }
    }

    /// Returns the tables that locate and describe the track's samples
    #[must_use]
    pub const fn stbl(&self) -> &SampleTableBox {
        &self.stbl
    }

    /// Returns the children no field of this box claims, in the order they came
    #[must_use]
    pub fn other_boxes(&self) -> &[AnyBox] {
        self.other_boxes.as_slice()
    }
}

impl BoxDefinition for MediaInformationBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"minf");
}

impl BoxDecode for MediaInformationBox {
    /// # Errors
    ///
    /// * The failures of [`boxes`]: a child does not frame as a box.
    /// * [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox): no `stbl`.
    /// * [`DuplicateBox`](isobmff_core::ErrorKind::DuplicateBox): more than one `stbl`.
    /// * Whatever the child reports, on the [`containers`](Error::containers) path: the `stbl` does
    ///   not decode.
    fn decode_payload(payload: &[u8]) -> Result<Self, Error> {
        let mut stbl_boxes = ChildBoxes::new();
        let mut other_boxes = OtherBoxes::new();

        for child in boxes(payload) {
            let child = child?;
            if child.header().box_type() == SampleTableBox::BOX_TYPE {
                stbl_boxes.push(child);
            } else {
                other_boxes.keep(child);
            }
        }

        Ok(Self {
            stbl: stbl_boxes.exactly_one()?,
            other_boxes,
        })
    }
}

impl BoxEncode for MediaInformationBox {
    fn payload_len(&self) -> u64 {
        let others = self
            .other_boxes
            .as_slice()
            .iter()
            .fold(0_u64, |total, other| {
                total.saturating_add(other.encoded_len())
            });

        self.stbl.encoded_len().saturating_add(others)
    }

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), Error> {
        let expected = self.payload_len();
        let actual = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        if actual != expected {
            return Err(Error::buffer_length_mismatch(expected, actual));
        }

        let mut rest = self.stbl.encode(buffer)?;
        for other in self.other_boxes.as_slice() {
            rest = other.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, BoxType, Error};

    use super::MediaInformationBox;
    use crate::stbl::SampleTableBox;
    use crate::stsd::SampleDescriptionBox;

    /// Media information holding only the sample table
    fn media_information() -> MediaInformationBox {
        MediaInformationBox::new(SampleTableBox::new(SampleDescriptionBox::new(Vec::new())))
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(media_information: &MediaInformationBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(media_information.payload_len()).unwrap()];
        media_information.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let payload = encoded_payload(&media_information());

        assert_eq!(
            MediaInformationBox::decode_payload(&payload).unwrap(),
            media_information()
        );
    }

    #[test]
    fn the_mandatory_children_this_box_has_no_field_for_are_kept_unread() {
        let payload = [
            vec![
                0, 0, 0, 0x14, b'v', b'm', b'h', b'd', 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
            encoded_payload(&media_information()),
        ]
        .concat();

        let media_information = MediaInformationBox::decode_payload(&payload).unwrap();

        assert_eq!(media_information.other_boxes().len(), 1);
        assert_eq!(
            encoded_payload(&media_information).len(),
            payload.len(),
            "the vmhd is written back, though after the stbl rather than before it"
        );
    }

    #[test]
    fn a_box_holding_no_sample_table_is_rejected() {
        assert_eq!(
            MediaInformationBox::decode_payload(b""),
            Err(Error::missing_mandatory_box(BoxType::compact(*b"stbl")))
        );
    }
}
