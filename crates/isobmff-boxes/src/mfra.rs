//! [`MovieFragmentRandomAccessBox`], the table that helps find the sync samples of movie fragments

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, Boxes, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::mfro::MovieFragmentRandomAccessOffsetBox;
use crate::tfra::{self, TrackFragmentRandomAccessBox};

/// [`MovieFragmentRandomAccessBox`] (`mfra`), ISO/IEC 14496-12 §8.8.9
///
/// A hint to where the sync samples of a file of movie fragments are, one
/// `TrackFragmentRandomAccessBox` for each track it gives them for; the
/// movie fragments themselves are definitive. It is usually placed at or
/// near the end of the file.
///
/// The box writes the `tfra`s, then the children of other types, which are
/// kept in [`other_boxes`](Self::other_boxes), and then `mfro`, whatever
/// order they were read in: the order of ISO/IEC 14496-12 Table 2, and
/// §8.8.11.1 places `mfro` last, so that the size it repeats is last in the
/// file where the box is. A `tfra` that does not read is kept in
/// [`other_boxes`](Self::other_boxes) too.
#[doc(alias = "mfra")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct MovieFragmentRandomAccessBox {
    /// Sync samples of each track the box gives them for, at most one box per track
    pub tfra: Vec<TrackFragmentRandomAccessBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
    /// Copy of the size of the box
    pub mfro: MovieFragmentRandomAccessOffsetBox,
}

impl MovieFragmentRandomAccessBox {
    /// Returns the entries the `tfra`s of the box whose payload is `payload` declare, counted before the box is read
    ///
    /// The sum of the `number_of_entry` of each `tfra`, ISO/IEC 14496-12
    /// §8.8.10, saturating at `u64::MAX`, read without allocating, so that a
    /// reader that bounds what it holds can refuse the box before reading
    /// it. The count ends at a child that does not split, and a `tfra` too
    /// short to hold its `number_of_entry` counts none, as reading the box
    /// keeps such a child in [`other_boxes`](Self::other_boxes).
    ///
    /// ```
    /// use isobmff_boxes::MovieFragmentRandomAccessBox;
    ///
    /// // Two `tfra`s, of 2 and 3 entries, the second without its entries
    /// let tfra = |number_of_entry: u8| [b"\0\0\0\x18tfra\0\0\0\0\0\0\0\x01\0\0\0\0\0\0\0".as_slice(), &[number_of_entry]].concat();
    /// let payload = [tfra(2), tfra(3)].concat();
    ///
    /// assert_eq!(MovieFragmentRandomAccessBox::entry_count(&payload), 5);
    /// ```
    #[must_use]
    pub fn entry_count(payload: &[u8]) -> u64 {
        Boxes::new(payload)
            .map_while(Result::ok)
            .filter(|child| child.header().box_type() == TrackFragmentRandomAccessBox::BOX_TYPE)
            .filter_map(|child| tfra::number_of_entry(child.payload()))
            .map(u64::from)
            .fold(0, u64::saturating_add)
    }
}

impl BoxDefinition for MovieFragmentRandomAccessBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mfra");
}

impl BoxDecode for MovieFragmentRandomAccessBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;
        let tfra = children.take_zero_or_more_if_decoded();
        let mfro = children.take_exactly_one()?;

        Ok(Self {
            tfra,
            other_boxes: OtherBoxes::from(children),
            mfro,
        })
    }
}

impl BoxEncode for MovieFragmentRandomAccessBox {
    fn payload_len(&self) -> u64 {
        self.tfra
            .iter()
            .map(BoxEncode::encoded_len)
            .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
            .fold(self.mfro.encoded_len(), u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = writer.take_remainder();
        for track_fragment_random_access in &self.tfra {
            rest = track_fragment_random_access.encode(rest)?;
        }
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }
        self.mfro.encode(rest)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxDefinition, ChildBoxes, Error, OtherBoxes};

    use super::MovieFragmentRandomAccessBox;
    use crate::encoded;
    use crate::mfro::MovieFragmentRandomAccessOffsetBox;
    use crate::tfra::TrackFragmentRandomAccessBox;

    /// `tfra` of version 0, track 1, 1-byte numbers and no entry
    const TFRA: &[u8] = b"\0\0\0\x18tfra\0\0\0\0\0\0\0\x01\0\0\0\0\0\0\0\0";

    /// `mfro` of a parent size of 56, the size of the `mfra` around it
    const MFRO: &[u8] = b"\0\0\0\x10mfro\0\0\0\0\0\0\0\x38";

    /// `free` holding nothing, a box `mfra` holds no field for
    const FREE: &[u8] = b"\0\0\0\x08free";

    fn random_access() -> MovieFragmentRandomAccessBox {
        MovieFragmentRandomAccessBox {
            tfra: vec![TrackFragmentRandomAccessBox::decode(TFRA).unwrap().0],
            other_boxes: OtherBoxes::from(ChildBoxes::decode(FREE).unwrap()),
            mfro: MovieFragmentRandomAccessOffsetBox { parent_size: 56 },
        }
    }

    #[test]
    fn reads_its_children_in_any_order_and_keeps_the_child_it_has_no_field_for() {
        let bytes = [b"\0\0\0\x38mfra".as_slice(), MFRO, FREE, TFRA].concat();

        assert_eq!(
            MovieFragmentRandomAccessBox::decode(&bytes),
            Ok((random_access(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_tfras_then_the_child_it_has_no_field_for_then_mfro_last() {
        assert_eq!(
            encoded(&random_access()),
            [b"\0\0\0\x38mfra".as_slice(), TFRA, FREE, MFRO].concat()
        );
    }

    #[test]
    fn a_box_of_no_mfro_is_refused() {
        assert_eq!(
            MovieFragmentRandomAccessBox::decode_payload(TFRA),
            Err(Error::missing_mandatory_box(
                MovieFragmentRandomAccessOffsetBox::BOX_TYPE
            ))
        );
    }

    #[test]
    fn the_entries_counted_are_those_the_tfras_declare_up_to_a_child_that_does_not_split() {
        let tfra = |number_of_entry: u8| {
            [
                b"\0\0\0\x18tfra\0\0\0\0\0\0\0\x01\0\0\0\0\0\0\0".as_slice(),
                &[number_of_entry],
            ]
            .concat()
        };
        let short = b"\0\0\0\x0ctfra\0\0\0\0".as_slice();
        let unsplit = b"\0\0\0\xfftfra".as_slice();
        let payload = [
            tfra(2),
            FREE.to_vec(),
            short.to_vec(),
            tfra(3),
            unsplit.to_vec(),
            tfra(7),
        ]
        .concat();

        assert_eq!(MovieFragmentRandomAccessBox::entry_count(&payload), 5);
    }
}
