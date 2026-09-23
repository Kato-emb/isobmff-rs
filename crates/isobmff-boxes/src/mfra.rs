//! [`MovieFragmentRandomAccessBox`] (`mfra`), ISO/IEC 14496-12 §8.8.9

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWidth, FieldWriter, OtherBoxes, boxes,
};

use crate::mfro::MovieFragmentRandomAccessOffsetBox;
use crate::tfra::TrackFragmentRandomAccessBox;

/// Box that gathers the random access tables of the tracks of a fragmented file
///
/// [`MovieFragmentRandomAccessBox`] (`mfra`), ISO/IEC 14496-12 §8.8.9. It
/// usually sits at the end of the file and holds at most one `tfra` per track;
/// a track without one may have every sample be a sync sample. The spec notes
/// that the table may be stale, so an entry is where to look rather than a
/// promise of what is there.
///
/// The `mfro` is not held: all it states is how many bytes this box occupies,
/// so [`encode_payload`](BoxEncode::encode_payload) derives it and writes it as
/// the last child, and [`decode_payload`](BoxDecode::decode_payload) requires
/// exactly one and reads its `size` as nothing.
///
/// On encode the `tfra` boxes are written first, then the children no field
/// claims, then the `mfro`, so a round-trip settles the order rather than
/// preserving it.
#[doc(alias = "mfra")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct MovieFragmentRandomAccessBox {
    tfra: Vec<TrackFragmentRandomAccessBox>,
    other_boxes: OtherBoxes,
}

impl MovieFragmentRandomAccessBox {
    /// Creates the box from the random access table of each track it covers
    #[must_use]
    pub const fn new(tfra: Vec<TrackFragmentRandomAccessBox>) -> Self {
        Self {
            tfra,
            other_boxes: OtherBoxes::new(),
        }
    }

    /// Returns the random access table of each track the box covers, in the order they came
    #[must_use]
    pub fn tfra(&self) -> &[TrackFragmentRandomAccessBox] {
        &self.tfra
    }

    /// Returns the children no field of this box claims, in the order they came
    #[must_use]
    pub fn other_boxes(&self) -> &[AnyBox] {
        self.other_boxes.as_slice()
    }
}

impl BoxDefinition for MovieFragmentRandomAccessBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mfra");
}

impl BoxDecode for MovieFragmentRandomAccessBox {
    /// # Errors
    ///
    /// * The failures of [`boxes`]: a child does not frame as a box.
    /// * [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox): no `mfro`.
    /// * [`DuplicateBox`](isobmff_core::ErrorKind::DuplicateBox): more than one `mfro`.
    /// * Whatever the child reports, on the [`containers`](Error::containers) path: a child does
    ///   not decode.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut tfra_boxes = ChildBoxes::new();
        let mut mfro_boxes = ChildBoxes::new();
        let mut other_boxes = OtherBoxes::new();

        for child in boxes(reader.take_remainder()) {
            let child = child?;
            let box_type = child.header().box_type();

            if box_type == TrackFragmentRandomAccessBox::BOX_TYPE {
                tfra_boxes.push(child);
            } else if box_type == MovieFragmentRandomAccessOffsetBox::BOX_TYPE {
                mfro_boxes.push(child);
            } else {
                other_boxes.keep(child);
            }
        }

        mfro_boxes.exactly_one::<MovieFragmentRandomAccessOffsetBox>()?;

        Ok(Self {
            tfra: tfra_boxes.zero_or_more()?,
            other_boxes,
        })
    }
}

impl BoxEncode for MovieFragmentRandomAccessBox {
    fn payload_len(&self) -> u64 {
        let random_access = self.tfra.iter().fold(0_u64, |total, tfra| {
            total.saturating_add(tfra.encoded_len())
        });
        let others = self
            .other_boxes
            .as_slice()
            .iter()
            .fold(0_u64, |total, other| {
                total.saturating_add(other.encoded_len())
            });
        let offset = MovieFragmentRandomAccessOffsetBox::new(0).encoded_len();

        random_access.saturating_add(others).saturating_add(offset)
    }

    /// # Errors
    ///
    /// * [`OutOfRange`](isobmff_core::ErrorKind::OutOfRange): the box occupies more
    ///   bytes than the 32-bit `size` of its `mfro` states.
    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = writer.take_remainder();
        for tfra in &self.tfra {
            rest = tfra.encode(rest)?;
        }
        for other in self.other_boxes.as_slice() {
            rest = other.encode(rest)?;
        }

        let encoded_len = self.encoded_len();
        let size = u32::try_from(encoded_len)
            .map_err(|_past_the_field| Error::out_of_range(encoded_len, FieldWidth::Compact))?;
        MovieFragmentRandomAccessOffsetBox::new(size).encode(rest)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxDefinition as _, BoxEncode, BoxType, Error, boxes};

    use super::MovieFragmentRandomAccessBox;
    use crate::mfro::MovieFragmentRandomAccessOffsetBox;
    use crate::tfra::TrackFragmentRandomAccessBox;
    use crate::tfra::tests::track_fragment_random_access;

    /// Writes the box and returns the bytes it occupies
    fn encoded(random_access: &MovieFragmentRandomAccessBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(random_access.encoded_len()).unwrap()];
        random_access.encode(&mut buffer).unwrap();

        buffer
    }

    /// Returns the payload of a box written whole
    fn payload_of(encoded: &[u8]) -> &[u8] {
        encoded.get(8..).unwrap()
    }

    #[test]
    fn the_last_child_is_an_offset_box_stating_the_bytes_the_box_occupies() {
        let random_access = MovieFragmentRandomAccessBox::new(vec![
            track_fragment_random_access(),
            TrackFragmentRandomAccessBox::new(2, Vec::new()),
        ]);

        let encoded = encoded(&random_access);
        let last_16_bytes = encoded.get(encoded.len() - 16..).unwrap();

        assert_eq!(
            MovieFragmentRandomAccessOffsetBox::decode(last_16_bytes).unwrap(),
            (
                MovieFragmentRandomAccessOffsetBox::new(u32::try_from(encoded.len()).unwrap()),
                &[][..]
            )
        );
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let random_access = MovieFragmentRandomAccessBox::new(vec![
            track_fragment_random_access(),
            TrackFragmentRandomAccessBox::new(2, Vec::new()),
        ]);

        let encoded = encoded(&random_access);

        assert_eq!(
            MovieFragmentRandomAccessBox::decode_payload(payload_of(&encoded)).unwrap(),
            random_access
        );
    }

    #[test]
    fn a_box_covering_no_track_reads_back_as_the_value_that_wrote_it() {
        let random_access = MovieFragmentRandomAccessBox::new(Vec::new());

        let encoded = encoded(&random_access);

        assert_eq!(encoded, b"\0\0\0\x18mfra\0\0\0\x10mfro\0\0\0\0\0\0\0\x18");
        assert_eq!(
            MovieFragmentRandomAccessBox::decode_payload(payload_of(&encoded)).unwrap(),
            random_access
        );
    }

    #[test]
    fn a_box_holding_no_offset_box_is_rejected() {
        assert_eq!(
            MovieFragmentRandomAccessBox::decode_payload(b""),
            Err(Error::missing_mandatory_box(BoxType::compact(*b"mfro")))
        );
    }

    #[test]
    fn the_children_this_box_has_no_field_for_are_kept_unread_ahead_of_the_offset_box() {
        let unknown = b"\0\0\0\x08free";
        let payload = [
            payload_of(&encoded(&MovieFragmentRandomAccessBox::new(Vec::new()))),
            unknown,
        ]
        .concat();

        let random_access = MovieFragmentRandomAccessBox::decode_payload(&payload).unwrap();
        let encoded = encoded(&random_access);
        let box_types: Vec<BoxType> = boxes(payload_of(&encoded))
            .map(|child| child.unwrap().header().box_type())
            .collect();

        assert_eq!(
            box_types,
            [
                BoxType::compact(*b"free"),
                MovieFragmentRandomAccessOffsetBox::BOX_TYPE,
            ]
        );
    }
}
