//! [`MovieFragmentBox`], a movie fragment that extends the presentation in time

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::meta::MetaBox;
use crate::mfhd::MovieFragmentHeaderBox;
use crate::traf::TrackFragmentBox;

/// [`MovieFragmentBox`] (`moof`), ISO/IEC 14496-12 §8.8.4
///
/// A movie fragment: the information about samples that, without fragments,
/// the `MovieBox` holds. It is a top-level box, a peer of the `MovieBox` and
/// the `MediaDataBox`es.
///
/// The box writes `mfhd`, `meta` and then the track fragments, whatever order
/// they were read in: the order of ISO/IEC 14496-12 §7.3.4 and Table 2.
/// Children of other types are kept in [`other_boxes`](Self::other_boxes), and
/// written back after them. A `meta` or a track fragment that does not read is
/// kept there too; a `meta` that does not read reads back [`None`].
#[doc(alias = "moof")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct MovieFragmentBox {
    /// Sequence number of the fragment
    pub mfhd: MovieFragmentHeaderBox,
    /// Untimed metadata and items of the fragment
    pub meta: Option<MetaBox>,
    /// Track fragments, zero or more per track, in the order they came
    pub traf: Vec<TrackFragmentBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for MovieFragmentBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"moof");
}

impl BoxDecode for MovieFragmentBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            mfhd: children.take_exactly_one()?,
            meta: children.take_zero_or_one_if_decoded()?,
            traf: children.take_zero_or_more_if_decoded(),
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for MovieFragmentBox {
    fn payload_len(&self) -> u64 {
        [
            self.mfhd.encoded_len(),
            self.meta.as_ref().map_or(0, BoxEncode::encoded_len),
        ]
        .into_iter()
        .chain(self.traf.iter().map(BoxEncode::encoded_len))
        .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
        .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = self.mfhd.encode(writer.take_remainder())?;
        if let Some(meta) = &self.meta {
            rest = meta.encode(rest)?;
        }
        for track_fragment in &self.traf {
            rest = track_fragment.encode(rest)?;
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

    use super::MovieFragmentBox;
    use crate::encoded;
    use crate::meta::MetaBox;
    use crate::mfhd::MovieFragmentHeaderBox;
    use crate::traf::TrackFragmentBox;

    /// `mfhd` of sequence number 7
    const MFHD: &[u8] = b"\0\0\0\x10mfhd\0\0\0\0\0\0\0\x07";

    /// `traf` of a `tfhd` of track 1 based at the `moof`, with a default size of 300 alone
    const TRAF: &[u8] = b"\0\0\0\x1ctraf\0\0\0\x14tfhd\0\x02\0\x10\0\0\0\x01\0\0\x01\x2c";

    /// `traf` of a `tfhd` of a version the box does not read
    const TRAF_UNREAD: &[u8] = b"\0\0\0\x18traf\0\0\0\x10tfhd\x01\0\0\0\0\0\0\x02";

    /// `meta` holding nothing
    const META: &[u8] = b"\0\0\0\x0cmeta\0\0\0\0";

    /// `pssh` holding nothing, a box `moof` holds no field for
    const PSSH: &[u8] = b"\0\0\0\x0cpssh\0\0\0\0";

    fn movie_fragment() -> MovieFragmentBox {
        let (track_fragment, _) = TrackFragmentBox::decode(TRAF).unwrap();

        MovieFragmentBox {
            mfhd: MovieFragmentHeaderBox { sequence_number: 7 },
            meta: Some(MetaBox::default()),
            traf: vec![track_fragment.clone(), track_fragment],
            other_boxes: OtherBoxes::from(ChildBoxes::decode(PSSH).unwrap()),
        }
    }

    #[test]
    fn reads_its_header_and_track_fragments_and_keeps_the_child_it_has_no_field_for() {
        let bytes = [b"\0\0\0\x68moof".as_slice(), TRAF, PSSH, META, MFHD, TRAF].concat();

        assert_eq!(
            MovieFragmentBox::decode(&bytes),
            Ok((movie_fragment(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_its_header_then_meta_then_its_track_fragments_then_the_child_it_has_no_field_for() {
        assert_eq!(
            encoded(&movie_fragment()),
            [b"\0\0\0\x68moof".as_slice(), MFHD, META, TRAF, TRAF, PSSH].concat()
        );
    }

    #[test]
    fn a_track_fragment_that_does_not_read_is_kept_as_its_bytes() {
        let bytes = [b"\0\0\0\x30moof".as_slice(), MFHD, TRAF_UNREAD].concat();
        let (read, _) = MovieFragmentBox::decode(&bytes).unwrap();

        assert_eq!((encoded(&read), read.traf), (bytes, vec![]));
    }

    #[test]
    fn a_box_of_no_mfhd_is_refused() {
        assert_eq!(
            MovieFragmentBox::decode_payload(TRAF),
            Err(Error::missing_mandatory_box(
                MovieFragmentHeaderBox::BOX_TYPE
            ))
        );
    }
}
