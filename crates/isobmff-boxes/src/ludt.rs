//! [`LoudnessBox`], the loudness declarations of an audio track

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::loudness::{AlbumLoudnessInfo, TrackLoudnessInfo};

/// [`LoudnessBox`] (`ludt`), ISO/IEC 14496-12 §12.2.7
///
/// Loudness declarations of an audio track, which lie in the `udta` of the
/// track so that a movie fragment can update them.
///
/// The box writes its `tlou` boxes, then its `alou` boxes, in the order of
/// the syntax of the class, whatever order they were read in. Children of
/// other types, and a `tlou` or `alou` that does not read, such as one of a
/// version other than 0, 1 or 2, are kept in
/// [`other_boxes`](Self::other_boxes), and written back after them.
///
/// At most one `tlou` and one `alou` of version 1 or 2 are allowed. A box of
/// more reads, but writing it is refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for `loudness` or
/// `albumLoudness`.
#[doc(alias = "ludt")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct LoudnessBox {
    /// Loudness characteristics of the track, in the order they came
    pub loudness: Vec<TrackLoudnessInfo>,
    /// Loudness characteristics of the album, in the order they came
    pub album_loudness: Vec<AlbumLoudnessInfo>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for LoudnessBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"ludt");
}

impl BoxDecode for LoudnessBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            loudness: children.take_zero_or_more_if_decoded(),
            album_loudness: children.take_zero_or_more_if_decoded(),
            other_boxes: OtherBoxes::from(children),
        })
    }
}

/// Returns whether more than one of `versions` is 1 or later, which a [`LoudnessBox`] allows one of for each type
fn more_than_one_of_version_1_or_later(versions: impl Iterator<Item = u8>) -> bool {
    versions.filter(|version| *version >= 1).count() > 1
}

impl BoxEncode for LoudnessBox {
    fn payload_len(&self) -> u64 {
        self.loudness
            .iter()
            .map(BoxEncode::encoded_len)
            .chain(self.album_loudness.iter().map(BoxEncode::encoded_len))
            .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
            .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if more_than_one_of_version_1_or_later(self.loudness.iter().map(|info| info.version)) {
            return Err(Error::invalid_value("loudness"));
        }
        if more_than_one_of_version_1_or_later(self.album_loudness.iter().map(|info| info.version))
        {
            return Err(Error::invalid_value("albumLoudness"));
        }
        let mut rest = writer.take_remainder();
        for loudness in &self.loudness {
            rest = loudness.encode(rest)?;
        }
        for album_loudness in &self.album_loudness {
            rest = album_loudness.encode(rest)?;
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

    use isobmff_core::{BoxDecode, BoxEncode, ChildBoxes, Error, OtherBoxes};

    use super::LoudnessBox;
    use crate::encoded;
    use crate::loudness::TrackLoudnessInfo;
    use crate::loudness::tests::{ALOU, TLOU, album_loudness, track_loudness};

    /// Box of a type `ludt` holds no field for
    const FREE: &[u8] = b"\0\0\0\x08free";

    #[test]
    fn a_loudness_box_writes_the_track_loudness_then_the_album_loudness_then_the_other_children() {
        let read = LoudnessBox::decode_payload(&[ALOU, FREE, TLOU].concat()).unwrap();
        let expected = LoudnessBox {
            loudness: vec![track_loudness()],
            album_loudness: vec![album_loudness()],
            other_boxes: OtherBoxes::from(ChildBoxes::decode(FREE).unwrap()),
        };

        assert_eq!(
            (encoded(&read), read),
            (
                [b"\0\0\0\x44ludt".as_slice(), TLOU, ALOU, FREE].concat(),
                expected
            )
        );
    }

    #[test]
    fn a_loudness_box_of_two_track_loudnesses_of_version_1_is_refused_when_written() {
        let version_1 = TrackLoudnessInfo {
            version: 1,
            ..TrackLoudnessInfo::default()
        };
        let loudness = LoudnessBox {
            loudness: vec![version_1.clone(), version_1],
            ..LoudnessBox::default()
        };

        assert_eq!(
            loudness.encode(&mut [0; 64]),
            Err(Error::invalid_value("loudness"))
        );
    }
}
