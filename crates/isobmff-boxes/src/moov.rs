//! [`MovieBox`], the structure data of a presentation

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::mvhd::MovieHeaderBox;
use crate::trak::TrackBox;

/// [`MovieBox`] (`moov`), ISO/IEC 14496-12 §8.2.1
///
/// Holds the structure data of a presentation: its header and its tracks.
///
/// Children of other types are kept in [`other_boxes`](Self::other_boxes),
/// and written back after `mvhd` and the tracks.
///
/// The box holds one or more tracks. A box of none does not read, and writing
/// it is refused with
/// [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox), so a
/// box built from [`Default`] is given a track before it is written.
#[doc(alias = "moov")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct MovieBox {
    /// Media-independent information of the presentation
    pub mvhd: MovieHeaderBox,
    /// Tracks of the presentation, in the order they came
    pub trak: Vec<TrackBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for MovieBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"moov");
}

impl BoxDecode for MovieBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            mvhd: children.take_exactly_one()?,
            trak: children.take_one_or_more()?,
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for MovieBox {
    fn payload_len(&self) -> u64 {
        [self.mvhd.encoded_len()]
            .into_iter()
            .chain(self.trak.iter().map(BoxEncode::encoded_len))
            .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
            .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.trak.is_empty() {
            return Err(Error::missing_mandatory_box(TrackBox::BOX_TYPE));
        }

        let mut rest = self.mvhd.encode(writer.take_remainder())?;
        for track in &self.trak {
            rest = track.encode(rest)?;
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

    use super::MovieBox;
    use crate::encoded;
    use crate::mvhd::MovieHeaderBox;
    use crate::trak::TrackBox;

    /// `mvhd` of version 0, timescale 1000, `next_track_ID` 2 and every other field zero or its template value
    const MVHD: &[u8] = b"\0\0\0\x6cmvhd\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x03\xe8\0\0\0\0\
        \0\x01\0\0\x01\0\0\0\0\0\0\0\0\0\0\0\
        \0\x01\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x01\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x40\0\0\0\
        \0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x02";

    /// `trak` of a `tkhd` of track 1 and an `mdia` holding the least it can
    const TRAK: &[u8] = b"\0\0\0\xd9trak\
        \0\0\0\x5ctkhd\0\0\0\x03\0\0\0\0\0\0\0\0\0\0\0\x01\0\0\0\0\0\0\0\0\
        \0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\
        \0\x01\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x01\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x40\0\0\0\
        \0\0\0\0\0\0\0\0\
        \0\0\0\x75mdia\
        \0\0\0\x20mdhd\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x03\xe8\0\0\0\0\x55\xc4\0\0\
        \0\0\0\x21hdlr\0\0\0\0\0\0\0\0vide\0\0\0\0\0\0\0\0\0\0\0\0\0\
        \0\0\0\x2cminf\0\0\0\x24dinf\0\0\0\x1cdref\0\0\0\0\0\0\0\x01\0\0\0\x0curl \0\0\0\x01";

    /// `iods` holding nothing, a box `moov` holds no field for
    const IODS: &[u8] = b"\0\0\0\x0ciods\0\0\0\0";

    fn movie() -> MovieBox {
        let (track, _) = TrackBox::decode(TRAK).unwrap();

        MovieBox {
            mvhd: MovieHeaderBox {
                timescale: 1000,
                next_track_id: 2,
                ..MovieHeaderBox::default()
            },
            trak: vec![track.clone(), track],
            other_boxes: OtherBoxes::from(ChildBoxes::decode(IODS).unwrap()),
        }
    }

    #[test]
    fn reads_its_header_and_tracks_in_order_and_keeps_the_child_it_has_no_field_for() {
        let bytes = [b"\0\0\x02\x32moov".as_slice(), MVHD, IODS, TRAK, TRAK].concat();

        assert_eq!(MovieBox::decode(&bytes), Ok((movie(), b"".as_slice())));
    }

    #[test]
    fn writes_its_header_then_its_tracks_then_the_child_it_has_no_field_for() {
        assert_eq!(
            encoded(&movie()),
            [b"\0\0\x02\x32moov".as_slice(), MVHD, TRAK, TRAK, IODS].concat()
        );
    }

    #[test]
    fn a_box_of_no_track_is_refused_when_read_and_when_written() {
        let mut no_track = movie();
        no_track.trak = vec![];

        assert_eq!(
            (
                MovieBox::decode_payload(MVHD),
                no_track.encode(&mut [0; 128])
            ),
            (
                Err(Error::missing_mandatory_box(TrackBox::BOX_TYPE)),
                Err(Error::missing_mandatory_box(TrackBox::BOX_TYPE))
            )
        );
    }
}
