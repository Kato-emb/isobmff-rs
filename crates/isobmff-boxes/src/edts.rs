//! [`EditBox`] (`edts`), ISO/IEC 14496-12 §8.6.5

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes, boxes,
};

use crate::elst::EditListBox;

/// Box that maps the media of a track onto the timeline of the movie
///
/// [`EditBox`] (`edts`), ISO/IEC 14496-12 §8.6.5. The `elst` it may hold is
/// promoted to a field of its own, and every other child is kept in
/// [`other_boxes`](Self::other_boxes) and written back unread. A track with no
/// `elst` maps its media onto the movie's timeline one to one.
///
/// On encode the `elst` is written first and then the children no field
/// claims, so a round-trip settles the order rather than preserving it.
#[doc(alias = "edts")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct EditBox {
    elst: Option<EditListBox>,
    other_boxes: OtherBoxes,
}

impl EditBox {
    /// Creates the box holding no edit list
    ///
    /// [`with_elst`](Self::with_elst) states the edit list.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            elst: None,
            other_boxes: OtherBoxes::new(),
        }
    }

    /// Sets the edit list that lays out the track's timeline
    #[must_use]
    pub fn with_elst(self, elst: EditListBox) -> Self {
        Self {
            elst: Some(elst),
            ..self
        }
    }

    /// Returns the edit list that lays out the track's timeline, if the box holds one
    #[must_use]
    pub const fn elst(&self) -> Option<&EditListBox> {
        self.elst.as_ref()
    }

    /// Returns the children no field of this box claims, in the order they came
    #[must_use]
    pub fn other_boxes(&self) -> &[AnyBox] {
        self.other_boxes.as_slice()
    }
}

impl Default for EditBox {
    fn default() -> Self {
        Self::new()
    }
}

impl BoxDefinition for EditBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"edts");
}

impl BoxDecode for EditBox {
    /// # Errors
    ///
    /// * The failures of [`boxes`]: a child does not frame as a box.
    /// * [`DuplicateBox`](isobmff_core::ErrorKind::DuplicateBox): more than one `elst`.
    /// * Whatever the child reports, on the [`containers`](Error::containers) path: the
    ///   `elst` does not decode.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut edit_list_boxes = ChildBoxes::new();
        let mut other_boxes = OtherBoxes::new();

        for child in boxes(reader.take_remainder()) {
            let child = child?;
            if child.header().box_type() == EditListBox::BOX_TYPE {
                edit_list_boxes.push(child);
            } else {
                other_boxes.keep(child);
            }
        }

        Ok(Self {
            elst: edit_list_boxes.zero_or_one()?,
            other_boxes,
        })
    }
}

impl BoxEncode for EditBox {
    fn payload_len(&self) -> u64 {
        let others = self
            .other_boxes
            .as_slice()
            .iter()
            .fold(0_u64, |total, other| {
                total.saturating_add(other.encoded_len())
            });

        self.elst
            .as_ref()
            .map_or(0, |elst| elst.encoded_len())
            .saturating_add(others)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = writer.take_remainder();
        if let Some(elst) = &self.elst {
            rest = elst.encode(rest)?;
        }
        for other in self.other_boxes.as_slice() {
            rest = other.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, BoxType, Error};

    use super::EditBox;
    use crate::elst::tests::edit_list;

    /// Edit box holding the edit list of a track starting 10 units into the movie
    pub(crate) fn edit() -> EditBox {
        EditBox::new().with_elst(edit_list())
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(edit: &EditBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(edit.payload_len()).unwrap()];
        edit.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_with_or_without_an_edit_list_reads_back_as_the_value_that_wrote_it() {
        for edit in [EditBox::new(), edit()] {
            let payload = encoded_payload(&edit);

            assert_eq!(EditBox::decode_payload(&payload).unwrap(), edit);
        }
    }

    #[test]
    fn a_second_edit_list_is_rejected() {
        let payload = encoded_payload(&edit()).repeat(2);

        assert_eq!(
            EditBox::decode_payload(&payload),
            Err(Error::duplicate_box(BoxType::compact(*b"elst")))
        );
    }
}
