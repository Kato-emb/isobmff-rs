//! [`MovieBox`] (`moov`), ISO/IEC 14496-12 §8.2.1

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, BoxWrite as _, ChildBoxes, Error,
    OtherBoxes, boxes,
};

use crate::mvex::MovieExtendsBox;
use crate::mvhd::MovieHeaderBox;
use crate::trak::TrackBox;

/// Box that holds every declaration a presentation is made of
///
/// [`MovieBox`] (`moov`), ISO/IEC 14496-12 §8.2.1. It carries the movie header,
/// one `trak` per track, and — when the presentation continues in fragments —
/// the `mvex` that says so.
///
/// On encode the children are written in the order the spec lists them —
/// `mvhd`, then the tracks, then `mvex` — and then the children no field
/// claims, so a round-trip settles the order rather than preserving it.
#[doc(alias = "moov")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct MovieBox {
    mvhd: MovieHeaderBox,
    trak: Vec<TrackBox>,
    mvex: Option<MovieExtendsBox>,
    other_boxes: OtherBoxes,
}

impl MovieBox {
    /// Creates the box from the movie header and the tracks it declares
    ///
    /// Returns `None` for an empty `trak`, which the spec does not allow: a
    /// presentation is made of at least one track.
    #[must_use]
    pub fn new(
        mvhd: MovieHeaderBox,
        trak: Vec<TrackBox>,
        mvex: Option<MovieExtendsBox>,
    ) -> Option<Self> {
        if trak.is_empty() {
            return None;
        }

        Some(Self {
            mvhd,
            trak,
            mvex,
            other_boxes: OtherBoxes::new(),
        })
    }

    /// Returns the declarations the presentation applies as a whole
    #[must_use]
    pub const fn mvhd(&self) -> &MovieHeaderBox {
        &self.mvhd
    }

    /// Returns the tracks the presentation is made of
    #[must_use]
    pub fn trak(&self) -> &[TrackBox] {
        &self.trak
    }

    /// Returns the declaration that the movie continues in fragments, if it does
    #[must_use]
    pub const fn mvex(&self) -> Option<&MovieExtendsBox> {
        self.mvex.as_ref()
    }

    /// Returns the children no field of this box claims, in the order they came
    #[must_use]
    pub fn other_boxes(&self) -> &[AnyBox] {
        self.other_boxes.as_slice()
    }
}

impl BoxDefinition for MovieBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"moov");
}

impl BoxDecode for MovieBox {
    /// # Errors
    ///
    /// * The failures of [`boxes`]: a child does not frame as a box.
    /// * [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox): no `mvhd`,
    ///   or no `trak` at all.
    /// * [`DuplicateBox`](isobmff_core::ErrorKind::DuplicateBox): more than one `mvhd` or
    ///   `mvex`.
    /// * Whatever the child reports, on the [`containers`](Error::containers) path: one of the
    ///   children does not decode.
    fn decode_payload(payload: &[u8]) -> Result<Self, Error> {
        let mut mvhd_boxes = ChildBoxes::new();
        let mut trak_boxes = ChildBoxes::new();
        let mut mvex_boxes = ChildBoxes::new();
        let mut other_boxes = OtherBoxes::new();

        for child in boxes(payload) {
            let child = child?;
            let box_type = child.header().box_type();

            if box_type == MovieHeaderBox::BOX_TYPE {
                mvhd_boxes.push(child);
            } else if box_type == TrackBox::BOX_TYPE {
                trak_boxes.push(child);
            } else if box_type == MovieExtendsBox::BOX_TYPE {
                mvex_boxes.push(child);
            } else {
                other_boxes.keep(child);
            }
        }

        Ok(Self {
            mvhd: mvhd_boxes.exactly_one()?,
            trak: trak_boxes.one_or_more()?,
            mvex: mvex_boxes.zero_or_one()?,
            other_boxes,
        })
    }
}

impl BoxEncode for MovieBox {
    fn payload_len(&self) -> u64 {
        let tracks = self.trak.iter().fold(0_u64, |total, track| {
            total.saturating_add(track.encoded_len())
        });
        let extends = self.mvex.as_ref().map_or(0, |mvex| mvex.encoded_len());
        let others = self
            .other_boxes
            .as_slice()
            .iter()
            .fold(0_u64, |total, other| {
                total.saturating_add(other.encoded_len())
            });

        self.mvhd
            .encoded_len()
            .saturating_add(tracks)
            .saturating_add(extends)
            .saturating_add(others)
    }

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), Error> {
        let expected = self.payload_len();
        let actual = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        if actual != expected {
            return Err(Error::buffer_length_mismatch(expected, actual));
        }

        let mut rest = self.mvhd.encode(buffer)?;
        for track in &self.trak {
            rest = track.encode(rest)?;
        }
        if let Some(mvex) = &self.mvex {
            rest = mvex.encode(rest)?;
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

    use isobmff_core::{BoxDecode, BoxEncode, BoxType, BoxWrite as _, Error};

    use super::MovieBox;
    use crate::mvex::tests::movie_extends;
    use crate::mvhd::tests::movie_header;
    use crate::trak::tests::track;

    /// Movie with one track, as a progressive file declares it
    fn movie() -> MovieBox {
        MovieBox::new(movie_header(5_000), vec![track()], None).unwrap()
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(movie: &MovieBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(movie.payload_len()).unwrap()];
        movie.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_movie_of_no_tracks_cannot_be_built() {
        assert_eq!(MovieBox::new(movie_header(0), Vec::new(), None), None);
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let payload = encoded_payload(&movie());

        assert_eq!(MovieBox::decode_payload(&payload).unwrap(), movie());
    }

    #[test]
    fn a_fragmented_movie_reads_back_with_its_extends_box() {
        let fragmented =
            MovieBox::new(movie_header(0), vec![track()], Some(movie_extends())).unwrap();

        let payload = encoded_payload(&fragmented);

        assert_eq!(MovieBox::decode_payload(&payload).unwrap(), fragmented);
    }

    #[test]
    fn a_movie_holding_no_track_is_rejected() {
        let whole = encoded_payload(&movie());
        let header_len = usize::try_from(movie().mvhd().encoded_len()).unwrap();

        assert_eq!(
            MovieBox::decode_payload(whole.get(..header_len).unwrap()),
            Err(Error::missing_mandatory_box(BoxType::compact(*b"trak")))
        );
    }

    #[test]
    fn a_second_movie_header_is_rejected() {
        let payload = [encoded_payload(&movie()), encoded_payload(&movie())].concat();

        assert_eq!(
            MovieBox::decode_payload(&payload),
            Err(Error::duplicate_box(BoxType::compact(*b"mvhd")))
        );
    }
}
