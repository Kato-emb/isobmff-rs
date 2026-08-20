//! [`MovieFragmentBox`] (`moof`), ISO/IEC 14496-12 §8.8.4

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, BoxWrite as _, ChildBoxes, Error,
    OtherBoxes, boxes,
};

use crate::mfhd::MovieFragmentHeaderBox;
use crate::traf::TrackFragmentBox;

/// Box that extends the presentation of a movie in time
///
/// [`MovieFragmentBox`] (`moof`), ISO/IEC 14496-12 §8.8.4. It carries what the
/// `moov` would have held for the samples that follow it, one `traf` per track
/// the fragment adds to; the samples themselves lie in the `mdat` beside it.
///
/// A fragment holding no `traf` at all reads and writes: §8.8.6 gives that child
/// a quantity of `Zero or more`.
///
/// On encode the children are written in the order the spec lists them — the
/// `mfhd`, then the track fragments — and then the children no field claims, so
/// a round-trip settles the order rather than preserving it.
#[doc(alias = "moof")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct MovieFragmentBox {
    mfhd: MovieFragmentHeaderBox,
    traf: Vec<TrackFragmentBox>,
    other_boxes: OtherBoxes,
}

impl MovieFragmentBox {
    /// Creates the box from its sequence number and the tracks it adds to
    #[must_use]
    pub const fn new(mfhd: MovieFragmentHeaderBox, traf: Vec<TrackFragmentBox>) -> Self {
        Self {
            mfhd,
            traf,
            other_boxes: OtherBoxes::new(),
        }
    }

    /// Returns the number this fragment carries against the other fragments
    #[must_use]
    pub const fn mfhd(&self) -> &MovieFragmentHeaderBox {
        &self.mfhd
    }

    /// Returns what this fragment adds to each track, in the order they came
    #[must_use]
    pub fn traf(&self) -> &[TrackFragmentBox] {
        &self.traf
    }

    /// Returns the children no field of this box claims, in the order they came
    #[must_use]
    pub fn other_boxes(&self) -> &[AnyBox] {
        self.other_boxes.as_slice()
    }
}

impl BoxDefinition for MovieFragmentBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"moof");
}

impl BoxDecode for MovieFragmentBox {
    /// # Errors
    ///
    /// * The failures of [`boxes`]: a child does not frame as a box.
    /// * [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox): no `mfhd`.
    /// * [`DuplicateBox`](isobmff_core::ErrorKind::DuplicateBox): more than one `mfhd`.
    /// * Whatever the child reports, on the [`containers`](Error::containers) path: a child does
    ///   not decode.
    fn decode_payload(payload: &[u8]) -> Result<Self, Error> {
        let mut mfhd_boxes = ChildBoxes::new();
        let mut traf_boxes = ChildBoxes::new();
        let mut other_boxes = OtherBoxes::new();

        for child in boxes(payload) {
            let child = child?;
            let box_type = child.header().box_type();

            if box_type == MovieFragmentHeaderBox::BOX_TYPE {
                mfhd_boxes.push(child);
            } else if box_type == TrackFragmentBox::BOX_TYPE {
                traf_boxes.push(child);
            } else {
                other_boxes.keep(child);
            }
        }

        Ok(Self {
            mfhd: mfhd_boxes.exactly_one()?,
            traf: traf_boxes.zero_or_more()?,
            other_boxes,
        })
    }
}

impl BoxEncode for MovieFragmentBox {
    fn payload_len(&self) -> u64 {
        let fragments = self.traf.iter().fold(0_u64, |total, traf| {
            total.saturating_add(traf.encoded_len())
        });
        let others = self
            .other_boxes
            .as_slice()
            .iter()
            .fold(0_u64, |total, other| {
                total.saturating_add(other.encoded_len())
            });

        self.mfhd
            .encoded_len()
            .saturating_add(fragments)
            .saturating_add(others)
    }

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), Error> {
        let expected = self.payload_len();
        let actual = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        if actual != expected {
            return Err(Error::buffer_length_mismatch(expected, actual));
        }

        let mut rest = self.mfhd.encode(buffer)?;
        for traf in &self.traf {
            rest = traf.encode(rest)?;
        }
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

    use isobmff_core::{BoxDecode, BoxDefinition as _, BoxEncode, BoxType, Error, boxes};

    use super::MovieFragmentBox;
    use crate::mfhd::MovieFragmentHeaderBox;
    use crate::traf::TrackFragmentBox;
    use crate::traf::tests::track_fragment;

    /// Movie fragment adding samples to the one track of a movie
    fn movie_fragment() -> MovieFragmentBox {
        MovieFragmentBox::new(MovieFragmentHeaderBox::new(1), vec![track_fragment(1)])
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(movie_fragment: &MovieFragmentBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(movie_fragment.payload_len()).unwrap()];
        movie_fragment.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let payload = encoded_payload(&movie_fragment());

        assert_eq!(
            MovieFragmentBox::decode_payload(&payload).unwrap(),
            movie_fragment()
        );
    }

    #[test]
    fn the_children_are_written_in_the_order_the_spec_lists_them() {
        let payload = encoded_payload(&movie_fragment());

        let box_types: Vec<BoxType> = boxes(&payload)
            .map(|child| child.unwrap().header().box_type())
            .collect();

        assert_eq!(
            box_types,
            [MovieFragmentHeaderBox::BOX_TYPE, TrackFragmentBox::BOX_TYPE]
        );
    }

    #[test]
    fn a_fragment_adding_to_no_track_reads_back_as_the_value_that_wrote_it() {
        let empty = MovieFragmentBox::new(MovieFragmentHeaderBox::new(1), Vec::new());

        let payload = encoded_payload(&empty);

        assert_eq!(MovieFragmentBox::decode_payload(&payload).unwrap(), empty);
    }

    #[test]
    fn every_track_fragment_is_kept_in_the_order_it_came() {
        let both = MovieFragmentBox::new(
            MovieFragmentHeaderBox::new(2),
            vec![track_fragment(1), track_fragment(2)],
        );

        let payload = encoded_payload(&both);

        assert_eq!(MovieFragmentBox::decode_payload(&payload).unwrap(), both);
    }

    #[test]
    fn the_children_this_box_has_no_field_for_are_kept_unread() {
        let payload = [
            encoded_payload(&movie_fragment()),
            vec![0, 0, 0, 0x08, b'f', b'r', b'e', b'e'],
        ]
        .concat();

        let movie_fragment = MovieFragmentBox::decode_payload(&payload).unwrap();

        assert_eq!(encoded_payload(&movie_fragment), payload);
    }

    #[test]
    fn a_box_holding_no_fragment_header_is_rejected() {
        assert_eq!(
            MovieFragmentBox::decode_payload(b""),
            Err(Error::missing_mandatory_box(BoxType::compact(*b"mfhd")))
        );
    }
}
