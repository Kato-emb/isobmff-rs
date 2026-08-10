//! [`SampleTableBox`] (`stbl`), ISO/IEC 14496-12 §8.5.1

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, BoxWrite as _, DecodeError, EncodeError,
    boxes,
};

use crate::container::{keep_unpromoted, promote_once, require, total_encoded_len, write_all};
use crate::stsd::SampleDescriptionBox;

/// Box that holds every table locating and describing the samples of a track
///
/// [`SampleTableBox`] (`stbl`), ISO/IEC 14496-12 §8.5.1. The `stsd` child is
/// promoted to a field of its own; the tables that give sample times, sizes,
/// and positions — `stts`, `stsc`, `stsz`, `stco` — have no fields yet, so they
/// are kept in [`other_boxes`](Self::other_boxes) and written back unread. The
/// spec marks those mandatory, and decoding does **not** enforce it: a `stbl`
/// missing them still decodes.
#[doc(alias = "stbl")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct SampleTableBox {
    stsd: SampleDescriptionBox,
    other_boxes: Vec<AnyBox>,
}

impl SampleTableBox {
    /// Creates the box from the sample description its samples are coded by
    #[must_use]
    pub const fn new(stsd: SampleDescriptionBox) -> Self {
        Self {
            stsd,
            other_boxes: Vec::new(),
        }
    }

    /// Returns the description of the coding every sample was made with
    #[must_use]
    pub const fn stsd(&self) -> &SampleDescriptionBox {
        &self.stsd
    }

    /// Returns the children no field of this box claims, in the order they came
    #[must_use]
    pub fn other_boxes(&self) -> &[AnyBox] {
        &self.other_boxes
    }
}

impl BoxDefinition for SampleTableBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"stbl");
}

impl BoxDecode for SampleTableBox {
    /// # Errors
    ///
    /// * [`Framing`](DecodeError::Framing): a child does not frame as a box.
    /// * [`MissingMandatoryBox`](DecodeError::MissingMandatoryBox): no `stsd`.
    /// * [`DuplicateBox`](DecodeError::DuplicateBox): more than one `stsd`.
    /// * [`Child`](DecodeError::Child): the `stsd` does not decode.
    fn decode_payload(payload: &[u8]) -> Result<Self, DecodeError> {
        let mut stsd = None;
        let mut other_boxes = Vec::new();

        for child in boxes(payload) {
            let child = child?;
            if child.header().box_type() == SampleDescriptionBox::BOX_TYPE {
                promote_once(&mut stsd, child)?;
            } else {
                keep_unpromoted(&mut other_boxes, child);
            }
        }

        Ok(Self {
            stsd: require(stsd)?,
            other_boxes,
        })
    }
}

impl BoxEncode for SampleTableBox {
    fn payload_len(&self) -> u64 {
        self.stsd
            .encoded_len()
            .saturating_add(total_encoded_len(&self.other_boxes))
    }

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), EncodeError> {
        let expected = self.payload_len();
        let actual = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        if actual != expected {
            return Err(EncodeError::BufferLengthMismatch { expected, actual });
        }

        let rest = self.stsd.encode(buffer)?;
        write_all(&self.other_boxes, rest)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{AnyBox, BoxDecode, BoxEncode, BoxType, DecodeError};

    use super::SampleTableBox;
    use crate::stsd::SampleDescriptionBox;

    /// Sample table holding only the description its samples are coded by
    fn sample_table() -> SampleTableBox {
        SampleTableBox::new(SampleDescriptionBox::new(Vec::new()))
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(sample_table: &SampleTableBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(sample_table.payload_len()).unwrap()];
        sample_table.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let payload = encoded_payload(&sample_table());

        assert_eq!(
            SampleTableBox::decode_payload(&payload).unwrap(),
            sample_table()
        );
    }

    #[test]
    fn a_table_no_field_claims_is_kept_and_written_back() {
        let payload = [
            encoded_payload(&sample_table()),
            vec![
                0, 0, 0, 0x10, b's', b't', b't', b's', 0, 0, 0, 0, 0, 0, 0, 0,
            ],
        ]
        .concat();

        let sample_table = SampleTableBox::decode_payload(&payload).unwrap();

        assert_eq!(
            sample_table.other_boxes().first().map(AnyBox::box_type),
            Some(BoxType::compact(*b"stts"))
        );
        assert_eq!(encoded_payload(&sample_table), payload);
    }

    #[test]
    fn a_box_holding_no_sample_description_is_rejected() {
        assert!(matches!(
            SampleTableBox::decode_payload(b""),
            Err(DecodeError::MissingMandatoryBox(_))
        ));
    }

    #[test]
    fn a_second_sample_description_is_rejected() {
        let payload = [
            encoded_payload(&sample_table()),
            encoded_payload(&sample_table()),
        ]
        .concat();

        assert!(matches!(
            SampleTableBox::decode_payload(&payload),
            Err(DecodeError::DuplicateBox(_))
        ));
    }
}
