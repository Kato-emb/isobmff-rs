//! [`MovieBox`] (`moov`), ISO/IEC 14496-12 §8.2.1

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes, boxes,
};

use crate::mdia::MediaBox;
use crate::mvex::MovieExtendsBox;
use crate::mvhd::MovieHeaderBox;
use crate::trak::TrackBox;
use crate::trex::TrackExtendsBox;

/// Box that holds every declaration a presentation is made of
///
/// [`MovieBox`] (`moov`), ISO/IEC 14496-12 §8.2.1. It carries the movie header,
/// one `trak` per track, and — when the presentation continues in fragments —
/// the `mvex` that says so.
///
/// §8.8.3 has one `trex` for each track of a fragmented movie, so this box
/// refuses a track left without one, in [`new`](Self::new) as in
/// [`decode_payload`](BoxDecode::decode_payload). A `track_id` identifies a
/// track over the whole presentation (§8.3.2.3), and that is held to in
/// [`new`](Self::new) alone: a decoded movie whose tracks collide on a
/// `track_id` reads as it came.
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
    ///
    /// Returns `None` when two tracks declare the same `track_id`, and — where
    /// the movie is fragmented — when a track is left without its `trex`.
    #[must_use]
    pub fn new(
        mvhd: MovieHeaderBox,
        trak: Vec<TrackBox>,
        mvex: Option<MovieExtendsBox>,
    ) -> Option<Self> {
        if trak.is_empty() {
            return None;
        }

        let declared_track_ids: BTreeSet<u32> =
            trak.iter().map(|track| track.tkhd().track_id()).collect();
        if declared_track_ids.len() != trak.len() {
            return None;
        }

        if a_track_lacks_its_trex(&trak, mvex.as_ref()) {
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

    /// Returns the media of the track `track_id` names, to be changed in place, or `None` for a track the movie does not declare
    ///
    /// The track's `tkhd` is not reached this way, so what [`new`](Self::new)
    /// settled — distinct `track_id`s, and a `trex` for each track where the
    /// movie is fragmented — still holds. A decoded movie whose tracks collide
    /// on `track_id` yields the first of them, in the order they came.
    #[must_use]
    pub fn mdia_mut(&mut self, track_id: u32) -> Option<&mut MediaBox> {
        self.trak
            .iter_mut()
            .find(|track| track.tkhd().track_id() == track_id)
            .map(TrackBox::mdia_mut)
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

/// Returns whether a track of the movie is left without its own `trex`
fn a_track_lacks_its_trex(trak: &[TrackBox], mvex: Option<&MovieExtendsBox>) -> bool {
    let Some(mvex) = mvex else {
        return false;
    };

    // Why not scanning the `trex` per track: both counts follow from the payload
    // length, so the scan would cost the product of two figures an input settles.
    let extended_track_ids: BTreeSet<u32> =
        mvex.trex().iter().map(TrackExtendsBox::track_id).collect();

    trak.iter()
        .any(|track| !extended_track_ids.contains(&track.tkhd().track_id()))
}

impl BoxDefinition for MovieBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"moov");
}

impl BoxDecode for MovieBox {
    /// # Errors
    ///
    /// * The failures of [`boxes`]: a child does not frame as a box.
    /// * [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox): no `mvhd`,
    ///   no `trak` at all, or a track of a fragmented movie without its `trex`.
    /// * [`DuplicateBox`](isobmff_core::ErrorKind::DuplicateBox): more than one `mvhd` or
    ///   `mvex`.
    /// * Whatever the child reports, on the [`containers`](Error::containers) path: one of the
    ///   children does not decode.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut mvhd_boxes = ChildBoxes::new();
        let mut trak_boxes = ChildBoxes::new();
        let mut mvex_boxes = ChildBoxes::new();
        let mut other_boxes = OtherBoxes::new();

        for child in boxes(reader.take_remainder()) {
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

        let mvhd = mvhd_boxes.exactly_one()?;
        let trak = trak_boxes.one_or_more()?;
        let mvex = mvex_boxes.zero_or_one()?;

        if a_track_lacks_its_trex(&trak, mvex.as_ref()) {
            return Err(Error::missing_mandatory_box(TrackExtendsBox::BOX_TYPE));
        }

        Ok(Self {
            mvhd,
            trak,
            mvex,
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

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = self.mvhd.encode(writer.take_remainder())?;
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

    use isobmff_core::{AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, Error};

    use super::{MovieBox, MovieExtendsBox, TrackExtendsBox};
    use crate::chunk_offset::ChunkOffsets;
    use crate::data_types::SampleFlags;
    use crate::mvex::tests::movie_extends;
    use crate::mvhd::tests::movie_header;
    use crate::sample_size::{SampleSizeBox, SampleSizes};
    use crate::stbl::SampleTableBox;
    use crate::stsc::SampleToChunkBox;
    use crate::stsd::SampleDescriptionBox;
    use crate::stts::TimeToSampleBox;
    use crate::trak::tests::track;

    /// Movie with one track, as a progressive file declares it
    fn movie() -> MovieBox {
        MovieBox::new(movie_header(5_000), vec![track()], None).unwrap()
    }

    /// Extends box setting the defaults of a track the movie does not declare
    fn extends_of_another_track() -> MovieExtendsBox {
        MovieExtendsBox::new(vec![TrackExtendsBox::new(7, 1, 0, 0, SampleFlags::ZERO)]).unwrap()
    }

    /// Writes one child whole and returns the bytes it occupies
    fn encoded_child(child: &(impl BoxDefinition + BoxEncode)) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(child.encoded_len()).unwrap()];
        child.encode(&mut buffer).unwrap();

        buffer
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
    fn a_movie_declaring_one_track_id_twice_cannot_be_built() {
        assert_eq!(
            MovieBox::new(movie_header(0), vec![track(), track()], None),
            None
        );
    }

    #[test]
    fn a_fragmented_movie_leaving_a_track_without_its_extends_box_cannot_be_built() {
        assert_eq!(
            MovieBox::new(
                movie_header(0),
                vec![track()],
                Some(extends_of_another_track())
            ),
            None
        );
    }

    #[test]
    fn a_fragmented_movie_leaving_a_track_without_its_extends_box_is_rejected() {
        let payload = [
            encoded_payload(&movie()),
            encoded_child(&extends_of_another_track()),
        ]
        .concat();

        assert_eq!(
            MovieBox::decode_payload(&payload),
            Err(Error::missing_mandatory_box(BoxType::compact(*b"trex")))
        );
    }

    #[test]
    fn a_decoded_movie_declaring_one_track_id_twice_keeps_both_tracks() {
        let payload = [encoded_payload(&movie()), encoded_child(&track())].concat();

        let decoded = MovieBox::decode_payload(&payload).unwrap();

        assert_eq!(decoded.trak(), [track(), track()]);
    }

    #[test]
    fn the_sample_tables_of_a_track_are_replaced_in_place_and_the_rest_of_the_movie_kept() {
        let unclaimed = vec![
            0, 0, 0, 0x0c, b'u', b'd', b't', b'a', 0x11, 0x11, 0x11, 0x11,
        ];
        let payload = [encoded_payload(&movie()), unclaimed].concat();
        let mut decoded = MovieBox::decode_payload(&payload).unwrap();
        let laid_out = SampleTableBox::new(
            SampleDescriptionBox::new(Vec::new()),
            TimeToSampleBox::from_deltas([3_000]),
            SampleToChunkBox::from_chunks([(1, 1)]).unwrap(),
            SampleSizes::Stsz(SampleSizeBox::from_sizes([4])),
            ChunkOffsets::from_offsets([1_000]),
        );

        *decoded.mdia_mut(1).unwrap().minf_mut().stbl_mut() = laid_out.clone();

        assert_eq!(
            decoded
                .trak()
                .first()
                .map(|track| track.mdia().minf().stbl()),
            Some(&laid_out)
        );
        assert_eq!(
            decoded.other_boxes(),
            [AnyBox::from_raw_bytes(
                BoxType::compact(*b"udta"),
                vec![0x11; 4]
            )]
        );
    }

    #[test]
    fn the_media_of_a_track_the_movie_does_not_declare_yields_nothing() {
        assert_eq!(movie().mdia_mut(7), None);
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
