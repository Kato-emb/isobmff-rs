//! [`MovieExtendsBox`] (`mvex`), ISO/IEC 14496-12 §8.8.1

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, BoxWrite as _, DecodeError, EncodeError,
    boxes,
};

use crate::container::{keep_unpromoted, promote_each, require_any, total_encoded_len, write_all};
use crate::trex::TrackExtendsBox;

/// Box whose presence says the movie continues in fragments
///
/// [`MovieExtendsBox`] (`mvex`), ISO/IEC 14496-12 §8.8.1. A reader that finds
/// one knows the `moov` does not hold every sample, and that `moof` boxes
/// follow. It carries one `trex` per track, setting the defaults each
/// fragment of that track falls back on.
///
/// A `mehd`, which states the fragmented duration up front, has no fields yet
/// and is kept in [`other_boxes`](Self::other_boxes).
#[doc(alias = "mvex")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct MovieExtendsBox {
    trex: Vec<TrackExtendsBox>,
    other_boxes: Vec<AnyBox>,
}

impl MovieExtendsBox {
    /// Creates the box from the per-track defaults it sets
    ///
    /// Returns `None` for an empty `trex`, which would leave the fragments of
    /// every track without the defaults the spec requires one of these to give.
    #[must_use]
    pub fn new(trex: Vec<TrackExtendsBox>) -> Option<Self> {
        if trex.is_empty() {
            return None;
        }

        Some(Self {
            trex,
            other_boxes: Vec::new(),
        })
    }

    /// Returns the defaults set for each track the movie declares
    #[must_use]
    pub fn trex(&self) -> &[TrackExtendsBox] {
        &self.trex
    }

    /// Returns the children no field of this box claims, in the order they came
    #[must_use]
    pub fn other_boxes(&self) -> &[AnyBox] {
        &self.other_boxes
    }
}

impl BoxDefinition for MovieExtendsBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mvex");
}

impl BoxDecode for MovieExtendsBox {
    /// # Errors
    ///
    /// * [`Framing`](DecodeError::Framing): a child does not frame as a box.
    /// * [`MissingMandatoryBox`](DecodeError::MissingMandatoryBox): no `trex`.
    /// * [`Child`](DecodeError::Child): a `trex` does not decode.
    fn decode_payload(payload: &[u8]) -> Result<Self, DecodeError> {
        let mut trex = Vec::new();
        let mut other_boxes = Vec::new();

        for child in boxes(payload) {
            let child = child?;
            if child.header().box_type() == TrackExtendsBox::BOX_TYPE {
                promote_each(&mut trex, child)?;
            } else {
                keep_unpromoted(&mut other_boxes, child);
            }
        }

        Ok(Self {
            trex: require_any(trex)?,
            other_boxes,
        })
    }
}

impl BoxEncode for MovieExtendsBox {
    fn payload_len(&self) -> u64 {
        self.trex
            .iter()
            .fold(0_u64, |total, trex| {
                total.saturating_add(trex.encoded_len())
            })
            .saturating_add(total_encoded_len(&self.other_boxes))
    }

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), EncodeError> {
        let expected = self.payload_len();
        let actual = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        if actual != expected {
            return Err(EncodeError::BufferLengthMismatch { expected, actual });
        }

        let mut rest = buffer;
        for trex in &self.trex {
            rest = trex.encode(rest)?;
        }
        write_all(&self.other_boxes, rest)?;

        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, DecodeError};

    use super::MovieExtendsBox;
    use crate::trex::TrackExtendsBox;

    /// Movie extends box declaring the defaults of one track
    pub(crate) fn movie_extends() -> MovieExtendsBox {
        MovieExtendsBox::new(vec![TrackExtendsBox::new(1, 1, 0, 0, 0)]).unwrap()
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(movie_extends: &MovieExtendsBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(movie_extends.payload_len()).unwrap()];
        movie_extends.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_declaring_no_track_defaults_cannot_be_built() {
        assert_eq!(MovieExtendsBox::new(Vec::new()), None);
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let payload = encoded_payload(&movie_extends());

        assert_eq!(
            MovieExtendsBox::decode_payload(&payload).unwrap(),
            movie_extends()
        );
    }

    #[test]
    fn every_track_default_is_kept_in_the_order_it_came() {
        let both = MovieExtendsBox::new(vec![
            TrackExtendsBox::new(1, 1, 0, 0, 0),
            TrackExtendsBox::new(2, 1, 1_024, 0, 0),
        ])
        .unwrap();

        let payload = encoded_payload(&both);

        assert_eq!(MovieExtendsBox::decode_payload(&payload).unwrap(), both);
    }

    #[test]
    fn a_box_holding_no_track_defaults_is_rejected() {
        assert!(matches!(
            MovieExtendsBox::decode_payload(b""),
            Err(DecodeError::MissingMandatoryBox(_))
        ));
    }
}
