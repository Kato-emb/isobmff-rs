//! [`SubTrackDefinitionBox`], the container of the definition of a sub track

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::stsg::SubTrackSampleGroupBox;

/// [`SubTrackDefinitionBox`] (`strd`), ISO/IEC 14496-12 §8.13.5
///
/// Holds the boxes that define a sub track: the sample groups it is made of,
/// or the media-specific definitions of a derived specification.
///
/// Children of other types, and an `stsg` that does not read, are kept in
/// [`other_boxes`](Self::other_boxes), and written back after the `stsg`
/// boxes.
#[doc(alias = "strd")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct SubTrackDefinitionBox {
    /// Sample groups the sub track is made of, in the order they came
    pub stsg: Vec<SubTrackSampleGroupBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for SubTrackDefinitionBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"strd");
}

impl BoxDecode for SubTrackDefinitionBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            stsg: children.take_zero_or_more_if_decoded(),
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for SubTrackDefinitionBox {
    fn payload_len(&self) -> u64 {
        self.stsg
            .iter()
            .map(BoxEncode::encoded_len)
            .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
            .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = writer.take_remainder();
        for sample_group in &self.stsg {
            rest = sample_group.encode(rest)?;
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

    use isobmff_core::{BoxDecode, ChildBoxes, OtherBoxes};

    use super::SubTrackDefinitionBox;
    use crate::encoded;
    use crate::stsg::SubTrackSampleGroupBox;

    /// `stsg` of the `tele` sample group of entry 1
    const STSG: &[u8] = b"\0\0\0\x16stsg\0\0\0\0tele\0\x01\0\0\0\x01";

    /// `stsg` of a version the box does not read
    const STSG_UNREAD: &[u8] = b"\0\0\0\x12stsg\x01\0\0\0tele\0\0";

    /// `free` holding nothing, a box `strd` holds no field for
    const FREE: &[u8] = b"\0\0\0\x08free";

    #[test]
    fn writes_its_sample_groups_then_the_children_it_has_no_field_for() {
        let bytes = [b"\0\0\0\x4estrd".as_slice(), FREE, STSG, STSG_UNREAD, STSG].concat();
        let (read, _) = SubTrackDefinitionBox::decode(&bytes).unwrap();
        let (sample_group, _) = SubTrackSampleGroupBox::decode(STSG).unwrap();

        assert_eq!(
            (read.clone(), encoded(&read)),
            (
                SubTrackDefinitionBox {
                    stsg: vec![sample_group.clone(), sample_group],
                    other_boxes: OtherBoxes::from(
                        ChildBoxes::decode(&[FREE, STSG_UNREAD].concat()).unwrap()
                    ),
                },
                [b"\0\0\0\x4estrd".as_slice(), STSG, STSG, FREE, STSG_UNREAD].concat()
            )
        );
    }
}
