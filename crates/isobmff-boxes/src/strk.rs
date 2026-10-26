//! [`SubTrackBox`], the container of the information of a sub track

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::strd::SubTrackDefinitionBox;
use crate::stri::SubTrackInformationBox;

/// [`SubTrackBox`] (`strk`), ISO/IEC 14496-12 §8.13.3
///
/// Holds the boxes that define, and give information about, one sub track of
/// the track whose `UserDataBox` holds the box.
///
/// The box holds exactly one `stri` and exactly one `strd`: a box without
/// either, or with one that does not read, is refused. The box writes `stri`
/// and then `strd`, whatever order they were read in: the order of ISO/IEC
/// 14496-12 Table 2. Children of other types are kept in
/// [`other_boxes`](Self::other_boxes), and written back after them.
#[doc(alias = "strk")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct SubTrackBox {
    /// Groups, identifier and attributes of the sub track
    pub stri: SubTrackInformationBox,
    /// Definition of the sub track
    pub strd: SubTrackDefinitionBox,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for SubTrackBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"strk");
}

impl BoxDecode for SubTrackBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            stri: children.take_exactly_one()?,
            strd: children.take_exactly_one()?,
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for SubTrackBox {
    fn payload_len(&self) -> u64 {
        self.other_boxes
            .as_slice()
            .iter()
            .map(AnyBox::encoded_len)
            .fold(
                self.stri
                    .encoded_len()
                    .saturating_add(self.strd.encoded_len()),
                u64::saturating_add,
            )
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = self.stri.encode(writer.take_remainder())?;
        rest = self.strd.encode(rest)?;
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, BoxDefinition, ChildBoxes, Error, OtherBoxes};

    use super::SubTrackBox;
    use crate::encoded;
    use crate::strd::SubTrackDefinitionBox;
    use crate::stri::SubTrackInformationBox;

    /// `stri` of sub track 2 and every other field zero
    const STRI: &[u8] = b"\0\0\0\x14stri\0\0\0\0\0\0\0\0\0\0\0\x02";

    /// `strd` of an `stsg` of the `tele` sample group of entry 1
    const STRD: &[u8] = b"\0\0\0\x1estrd\0\0\0\x16stsg\0\0\0\0tele\0\x01\0\0\0\x01";

    /// `free` holding nothing, a box `strk` holds no field for
    const FREE: &[u8] = b"\0\0\0\x08free";

    fn sub_track() -> SubTrackBox {
        SubTrackBox {
            stri: SubTrackInformationBox {
                sub_track_id: 2,
                ..SubTrackInformationBox::default()
            },
            strd: SubTrackDefinitionBox::decode(STRD).unwrap().0,
            other_boxes: OtherBoxes::from(ChildBoxes::decode(FREE).unwrap()),
        }
    }

    #[test]
    fn reads_its_children_in_any_order_and_keeps_the_child_it_has_no_field_for() {
        let bytes = [b"\0\0\0\x42strk".as_slice(), FREE, STRD, STRI].concat();

        assert_eq!(
            SubTrackBox::decode(&bytes),
            Ok((sub_track(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_stri_then_strd_then_the_child_it_has_no_field_for() {
        assert_eq!(
            encoded(&sub_track()),
            [b"\0\0\0\x42strk".as_slice(), STRI, STRD, FREE].concat()
        );
    }

    #[test]
    fn a_box_without_a_sub_track_definition_is_refused() {
        assert_eq!(
            SubTrackBox::decode_payload(STRI),
            Err(Error::missing_mandatory_box(
                SubTrackDefinitionBox::BOX_TYPE
            ))
        );
    }

    #[test]
    fn a_box_without_a_sub_track_information_is_refused() {
        assert_eq!(
            SubTrackBox::decode_payload(STRD),
            Err(Error::missing_mandatory_box(
                SubTrackInformationBox::BOX_TYPE
            ))
        );
    }
}
