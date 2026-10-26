//! [`MovieExtendsBox`], the warning that a file holds movie fragments

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::leva::LevelAssignmentBox;
use crate::mehd::MovieExtendsHeaderBox;
use crate::trep::TrackExtensionPropertiesBox;
use crate::trex::TrackExtendsBox;

/// [`MovieExtendsBox`] (`mvex`), ISO/IEC 14496-12 §8.8.1
///
/// Warns readers that the file can hold movie fragments, which they scan in
/// order and add to what the `MovieBox` holds, and sets the defaults those
/// fragments use.
///
/// The box writes `mehd`, the `trex`es, `leva` and the `trep`s in this
/// order, whatever order they were read in: the order of ISO/IEC 14496-12
/// Table 2. `trep`, which Table 2 does not list, is written last. Children of
/// other types are kept in [`other_boxes`](Self::other_boxes), and written
/// back after them. A `mehd`, a `leva` or a `trep` that does not read is kept
/// there too; the field of a `mehd` or a `leva` reads back [`None`].
///
/// The box holds one `trex` for each track. A box of none does not read, and
/// writing it is refused with
/// [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox), so a
/// box built from [`Default`] is given a `trex` before it is written.
#[doc(alias = "mvex")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct MovieExtendsBox {
    /// Overall duration of the fragmented movie
    pub mehd: Option<MovieExtendsHeaderBox>,
    /// Defaults the movie fragments of each track use, one per track
    pub trex: Vec<TrackExtendsBox>,
    /// Levels the movie fragments are assigned to
    pub leva: Option<LevelAssignmentBox>,
    /// Characteristics of tracks in the movie fragments, at most one per track
    pub trep: Vec<TrackExtensionPropertiesBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for MovieExtendsBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mvex");
}

impl BoxDecode for MovieExtendsBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            mehd: children.take_zero_or_one_if_decoded()?,
            trex: children.take_one_or_more()?,
            leva: children.take_zero_or_one_if_decoded()?,
            trep: children.take_zero_or_more_if_decoded(),
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for MovieExtendsBox {
    fn payload_len(&self) -> u64 {
        [
            self.mehd.as_ref().map_or(0, BoxEncode::encoded_len),
            self.leva.as_ref().map_or(0, BoxEncode::encoded_len),
        ]
        .into_iter()
        .chain(self.trex.iter().map(BoxEncode::encoded_len))
        .chain(self.trep.iter().map(BoxEncode::encoded_len))
        .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
        .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.trex.is_empty() {
            return Err(Error::missing_mandatory_box(TrackExtendsBox::BOX_TYPE));
        }

        let mut rest = writer.take_remainder();
        if let Some(mehd) = &self.mehd {
            rest = mehd.encode(rest)?;
        }
        for track_extends in &self.trex {
            rest = track_extends.encode(rest)?;
        }
        if let Some(leva) = &self.leva {
            rest = leva.encode(rest)?;
        }
        for properties in &self.trep {
            rest = properties.encode(rest)?;
        }
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, ChildBoxes, Error, OtherBoxes};

    use super::MovieExtendsBox;
    use crate::encoded;
    use crate::leva::LevelAssignmentBox;
    use crate::mehd::MovieExtendsHeaderBox;
    use crate::trep::TrackExtensionPropertiesBox;
    use crate::trex::TrackExtendsBox;

    /// `mehd` of version 0 and a duration of 90000
    const MEHD: &[u8] = b"\0\0\0\x10mehd\0\0\0\0\0\x01\x5f\x90";

    /// `trex` of track 1, description 1 and every other field zero
    const TREX: &[u8] = b"\0\0\0\x20trex\0\0\0\0\0\0\0\x01\0\0\0\x01\0\0\0\0\0\0\0\0\0\0\0\0";

    /// `leva` of two levels, tracks 1 and 2 by track (type 2)
    const LEVA: &[u8] = b"\0\0\0\x17leva\0\0\0\0\x02\0\0\0\x01\x02\0\0\0\x02\x02";

    /// `trep` of track 1 holding nothing
    const TREP: &[u8] = b"\0\0\0\x10trep\0\0\0\0\0\0\0\x01";

    /// `free` holding nothing, a box `mvex` holds no field for
    const FREE: &[u8] = b"\0\0\0\x08free";

    fn movie_extends() -> MovieExtendsBox {
        MovieExtendsBox {
            mehd: Some(MovieExtendsHeaderBox::decode(MEHD).unwrap().0),
            trex: vec![TrackExtendsBox::decode(TREX).unwrap().0],
            leva: Some(LevelAssignmentBox::decode(LEVA).unwrap().0),
            trep: vec![TrackExtensionPropertiesBox::decode(TREP).unwrap().0],
            other_boxes: OtherBoxes::from(ChildBoxes::decode(FREE).unwrap()),
        }
    }

    #[test]
    fn reads_its_children_in_any_order_and_keeps_the_child_it_has_no_field_for() {
        let bytes = [b"\0\0\0\x67mvex".as_slice(), FREE, TREP, LEVA, TREX, MEHD].concat();

        assert_eq!(
            MovieExtendsBox::decode(&bytes),
            Ok((movie_extends(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_its_children_in_the_order_of_table_2_then_the_child_it_has_no_field_for() {
        assert_eq!(
            encoded(&movie_extends()),
            [b"\0\0\0\x67mvex".as_slice(), MEHD, TREX, LEVA, TREP, FREE].concat()
        );
    }

    #[test]
    fn a_box_of_no_trex_is_refused_when_read_and_when_written() {
        let mut no_trex = movie_extends();
        no_trex.trex = vec![];

        assert_eq!(
            (
                MovieExtendsBox::decode_payload(MEHD),
                no_trex.encode(&mut [0; 128])
            ),
            (
                Err(Error::missing_mandatory_box(TrackExtendsBox::BOX_TYPE)),
                Err(Error::missing_mandatory_box(TrackExtendsBox::BOX_TYPE))
            )
        );
    }
}
