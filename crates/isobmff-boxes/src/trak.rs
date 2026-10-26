//! [`TrackBox`], the container of a single track

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::edts::EditBox;
use crate::mdia::MediaBox;
use crate::meta::MetaBox;
use crate::tkhd::TrackHeaderBox;
use crate::tref::TrackReferenceBox;
use crate::trgr::TrackGroupBox;
use crate::ttyp::TrackTypeBox;
use crate::udta::UserDataBox;

/// [`TrackBox`] (`trak`), ISO/IEC 14496-12 §8.3.1
///
/// Holds a single track of a presentation, with its temporal and spatial
/// information and its media.
///
/// The box writes `tkhd`, `ttyp`, `tref`, `trgr`, `edts`, `meta`, `mdia` and
/// the `udta`s in this order, whatever order they were read in: the order of
/// ISO/IEC 14496-12 §7.3.4, and of Table 2 where §7.3.4 sets none. Children of
/// other types are kept in [`other_boxes`](Self::other_boxes), and written
/// back after `mdia` and before the `udta`s, which §7.3.4 places last. A
/// `ttyp`, a `tref`, a `trgr`, an `edts` or a `meta` that does not read is
/// kept there too, and reads back [`None`]. A `udta` that does not read is
/// kept there too.
#[doc(alias = "trak")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct TrackBox {
    /// Characteristics of the track
    pub tkhd: TrackHeaderBox,
    /// Brands the track complies with
    pub ttyp: Option<TrackTypeBox>,
    /// References of the track to other tracks and track groups
    pub tref: Option<TrackReferenceBox>,
    /// Track groups the track belongs to
    pub trgr: Option<TrackGroupBox>,
    /// Map of the presentation timeline to the media timeline
    pub edts: Option<EditBox>,
    /// Untimed metadata and items of the track
    pub meta: Option<MetaBox>,
    /// Media of the track
    pub mdia: MediaBox,
    /// User data of the track, in the order they came
    pub udta: Vec<UserDataBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for TrackBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"trak");
}

impl BoxDecode for TrackBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            tkhd: children.take_exactly_one()?,
            ttyp: children.take_zero_or_one_if_decoded()?,
            tref: children.take_zero_or_one_if_decoded()?,
            trgr: children.take_zero_or_one_if_decoded()?,
            edts: children.take_zero_or_one_if_decoded()?,
            meta: children.take_zero_or_one_if_decoded()?,
            mdia: children.take_exactly_one()?,
            udta: children.take_zero_or_more_if_decoded(),
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for TrackBox {
    fn payload_len(&self) -> u64 {
        [
            self.tkhd.encoded_len(),
            self.ttyp.as_ref().map_or(0, BoxEncode::encoded_len),
            self.tref.as_ref().map_or(0, BoxEncode::encoded_len),
            self.trgr.as_ref().map_or(0, BoxEncode::encoded_len),
            self.edts.as_ref().map_or(0, BoxEncode::encoded_len),
            self.meta.as_ref().map_or(0, BoxEncode::encoded_len),
            self.mdia.encoded_len(),
        ]
        .into_iter()
        .chain(self.udta.iter().map(BoxEncode::encoded_len))
        .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
        .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = self.tkhd.encode(writer.take_remainder())?;
        if let Some(ttyp) = &self.ttyp {
            rest = ttyp.encode(rest)?;
        }
        if let Some(tref) = &self.tref {
            rest = tref.encode(rest)?;
        }
        if let Some(trgr) = &self.trgr {
            rest = trgr.encode(rest)?;
        }
        if let Some(edts) = &self.edts {
            rest = edts.encode(rest)?;
        }
        if let Some(meta) = &self.meta {
            rest = meta.encode(rest)?;
        }
        rest = self.mdia.encode(rest)?;
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }
        for user_data in &self.udta {
            rest = user_data.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxDefinition, ChildBoxes, Error, OtherBoxes};

    use super::TrackBox;
    use crate::edts::EditBox;
    use crate::encoded;
    use crate::mdia::MediaBox;
    use crate::meta::MetaBox;
    use crate::tkhd::TrackHeaderBox;
    use crate::tref::TrackReferenceBox;
    use crate::trgr::TrackGroupBox;
    use crate::udta::UserDataBox;

    /// `tkhd` of version 0, enabled and in the movie, of track 1 and every other field zero or its template value
    const TKHD: &[u8] = b"\0\0\0\x5ctkhd\0\0\0\x03\0\0\0\0\0\0\0\0\0\0\0\x01\0\0\0\0\0\0\0\0\
        \0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\
        \0\x01\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x01\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x40\0\0\0\
        \0\0\0\0\0\0\0\0";

    /// `edts` holding nothing
    const EDTS: &[u8] = b"\0\0\0\x08edts";

    /// `mdia` of an `mdhd`, an `hdlr` and a `minf`, each holding the least it can
    const MDIA: &[u8] = b"\0\0\0\x8dmdia\
        \0\0\0\x20mdhd\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x03\xe8\0\0\0\0\x55\xc4\0\0\
        \0\0\0\x21hdlr\0\0\0\0\0\0\0\0vide\0\0\0\0\0\0\0\0\0\0\0\0\0\
        \0\0\0\x44minf\0\0\0\x24dinf\0\0\0\x1cdref\0\0\0\0\0\0\0\x01\0\0\0\x0curl \0\0\0\x01\0\0\0\x18stbl\0\0\0\x10stsd\0\0\0\0\0\0\0\0";

    /// `tref` of a `hint` reference to the track 2
    const TREF: &[u8] = b"\0\0\0\x14tref\0\0\0\x0chint\0\0\0\x02";

    /// `trgr` of the `msrc` track group 1
    const TRGR: &[u8] = b"\0\0\0\x18trgr\0\0\0\x10msrc\0\0\0\0\0\0\0\x01";

    /// `meta` holding nothing
    const META: &[u8] = b"\0\0\0\x0cmeta\0\0\0\0";

    /// `udta` of a `tsel` of switch group 1 and no attribute
    const UDTA: &[u8] = b"\0\0\0\x18udta\0\0\0\x10tsel\0\0\0\0\0\0\0\x01";

    /// `free` holding nothing, a box `trak` holds no field for
    const FREE: &[u8] = b"\0\0\0\x08free";

    fn track() -> TrackBox {
        TrackBox {
            tkhd: TrackHeaderBox {
                track_enabled: true,
                track_in_movie: true,
                track_id: 1,
                ..TrackHeaderBox::default()
            },
            ttyp: None,
            tref: Some(TrackReferenceBox::decode(TREF).unwrap().0),
            trgr: Some(TrackGroupBox::decode(TRGR).unwrap().0),
            edts: Some(EditBox::default()),
            meta: Some(MetaBox::default()),
            mdia: MediaBox::decode(MDIA).unwrap().0,
            udta: vec![UserDataBox::decode(UDTA).unwrap().0],
            other_boxes: OtherBoxes::from(ChildBoxes::decode(FREE).unwrap()),
        }
    }

    #[test]
    fn reads_its_children_and_keeps_the_child_it_has_no_field_for() {
        let bytes = [
            b"\0\0\x01\x51trak".as_slice(),
            TKHD,
            UDTA,
            META,
            EDTS,
            TRGR,
            FREE,
            MDIA,
            TREF,
        ]
        .concat();

        assert_eq!(TrackBox::decode(&bytes), Ok((track(), b"".as_slice())));
    }

    #[test]
    fn writes_its_children_in_the_order_of_their_fields_then_the_child_it_has_no_field_for_then_udta()
     {
        assert_eq!(
            encoded(&track()),
            [
                b"\0\0\x01\x51trak".as_slice(),
                TKHD,
                TREF,
                TRGR,
                EDTS,
                META,
                MDIA,
                FREE,
                UDTA
            ]
            .concat()
        );
    }

    #[test]
    fn a_child_it_has_no_field_for_read_after_udta_is_written_before_it() {
        let payload = [TKHD, MDIA, UDTA, FREE].concat();
        let track = TrackBox::decode_payload(&payload).unwrap();

        assert_eq!(
            encoded(&track),
            [b"\0\0\x01\x11trak".as_slice(), TKHD, MDIA, FREE, UDTA].concat()
        );
    }

    #[test]
    fn writes_its_children_in_the_order_of_the_specification_whatever_order_they_were_read_in() {
        let payload = [MDIA, EDTS, TKHD].concat();
        let track = TrackBox::decode_payload(&payload).unwrap();

        assert_eq!(
            encoded(&track),
            [b"\0\0\0\xf9trak".as_slice(), TKHD, EDTS, MDIA].concat()
        );
    }

    #[test]
    fn a_track_type_of_a_brand_list_that_does_not_read_is_kept_among_the_other_children() {
        let unreadable = b"\0\0\0\x0attypis";
        let payload = [TKHD, MDIA, unreadable].concat();
        let mut expected = track();
        expected.tref = None;
        expected.trgr = None;
        expected.edts = None;
        expected.meta = None;
        expected.udta = vec![];
        expected.other_boxes = OtherBoxes::from(ChildBoxes::decode(unreadable).unwrap());

        assert_eq!(TrackBox::decode_payload(&payload), Ok(expected));
    }

    #[test]
    fn a_box_without_a_track_header_is_refused() {
        assert_eq!(
            TrackBox::decode_payload(MDIA),
            Err(Error::missing_mandatory_box(TrackHeaderBox::BOX_TYPE))
        );
    }
}
