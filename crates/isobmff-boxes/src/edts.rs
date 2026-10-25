//! [`EditBox`], the container of the edit list of a track

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::elst::EditListBox;

/// [`EditBox`] (`edts`), ISO/IEC 14496-12 §8.6.5
///
/// Maps the presentation timeline to the media timeline as it is stored in
/// the file, through the edit list it holds.
///
/// Children of other types are kept in [`other_boxes`](Self::other_boxes),
/// and written back after `elst`. An `elst` that does not read is kept there
/// too, and `elst` reads back [`None`].
#[doc(alias = "edts")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct EditBox {
    /// Edit list of the track
    pub elst: Option<EditListBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for EditBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"edts");
}

impl BoxDecode for EditBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            elst: children.take_zero_or_one_if_decoded()?,
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for EditBox {
    fn payload_len(&self) -> u64 {
        self.other_boxes
            .as_slice()
            .iter()
            .map(AnyBox::encoded_len)
            .fold(
                self.elst.as_ref().map_or(0, BoxEncode::encoded_len),
                u64::saturating_add,
            )
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = writer.take_remainder();
        if let Some(elst) = &self.elst {
            rest = elst.encode(rest)?;
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

    use isobmff_core::{BoxDecode, BoxDefinition, ChildBoxes, Error, OtherBoxes};

    use super::EditBox;
    use crate::elst::{EditListBox, EditListEntry};
    use crate::encoded;

    /// `elst` of one edit: 30000 of the media from 0 at rate 1.0
    const ELST: &[u8] = b"\0\0\0\x1celst\0\0\0\0\0\0\0\x01\0\0\x75\x30\0\0\0\0\0\x01\0\0";

    /// Box of a type `edts` holds no field for
    const FREE: &[u8] = b"\0\0\0\x08free";

    fn edit_list() -> EditListBox {
        EditListBox {
            version: 0,
            repeat_edits: false,
            entries: vec![EditListEntry {
                edit_duration: 30000,
                media_time: 0,
                media_rate_integer: 1,
                media_rate_fraction: 0,
            }],
        }
    }

    #[test]
    fn reads_its_edit_list_and_keeps_the_child_it_has_no_field_for() {
        let bytes = [b"\0\0\0\x2cedts".as_slice(), FREE, ELST].concat();

        assert_eq!(
            EditBox::decode(&bytes),
            Ok((
                EditBox {
                    elst: Some(edit_list()),
                    other_boxes: OtherBoxes::from(ChildBoxes::decode(FREE).unwrap()),
                },
                b"".as_slice()
            ))
        );
    }

    #[test]
    fn writes_its_edit_list_and_then_the_child_it_has_no_field_for() {
        let edit = EditBox {
            elst: Some(edit_list()),
            other_boxes: OtherBoxes::from(ChildBoxes::decode(FREE).unwrap()),
        };

        assert_eq!(
            encoded(&edit),
            [b"\0\0\0\x2cedts".as_slice(), ELST, FREE].concat()
        );
    }

    #[test]
    fn an_edit_list_of_a_version_it_does_not_read_is_kept_among_the_other_children() {
        let version_2 = b"\0\0\0\x10elst\x02\0\0\0\0\0\0\0";

        assert_eq!(
            EditBox::decode_payload(version_2),
            Ok(EditBox {
                elst: None,
                other_boxes: OtherBoxes::from(ChildBoxes::decode(version_2).unwrap()),
            })
        );
    }

    #[test]
    fn a_second_edit_list_is_refused() {
        assert_eq!(
            EditBox::decode_payload(&[ELST, ELST].concat()),
            Err(Error::duplicate_box(EditListBox::BOX_TYPE))
        );
    }
}
