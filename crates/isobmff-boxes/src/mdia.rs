//! [`MediaBox`], the declaration of the media of a track

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::elng::ExtendedLanguageBox;
use crate::hdlr::HandlerBox;
use crate::mdhd::MediaHeaderBox;
use crate::minf::MediaInformationBox;

/// [`MediaBox`] (`mdia`), ISO/IEC 14496-12 §8.4.1
///
/// Holds the boxes that declare information about the media data of a track.
///
/// Children of other types are kept in [`other_boxes`](Self::other_boxes),
/// and written back after `mdhd`, `hdlr`, `elng` and `minf`. An `elng` that
/// does not read is kept there too, and `elng` reads back [`None`].
#[doc(alias = "mdia")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct MediaBox {
    /// Media-independent information of the media
    pub mdhd: MediaHeaderBox,
    /// Media type of the track
    pub hdlr: HandlerBox,
    /// Language of the media as a BCP 47 tag
    pub elng: Option<ExtendedLanguageBox>,
    /// Characteristic information of the media
    pub minf: MediaInformationBox,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for MediaBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mdia");
}

impl BoxDecode for MediaBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            mdhd: children.take_exactly_one()?,
            hdlr: children.take_exactly_one()?,
            elng: children.take_zero_or_one_if_decoded()?,
            minf: children.take_exactly_one()?,
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for MediaBox {
    fn payload_len(&self) -> u64 {
        [
            self.mdhd.encoded_len(),
            self.hdlr.encoded_len(),
            self.elng.as_ref().map_or(0, BoxEncode::encoded_len),
            self.minf.encoded_len(),
        ]
        .into_iter()
        .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
        .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = self.mdhd.encode(writer.take_remainder())?;
        rest = self.hdlr.encode(rest)?;
        if let Some(elng) = &self.elng {
            rest = elng.encode(rest)?;
        }
        rest = self.minf.encode(rest)?;
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use isobmff_core::{
        BoxDecode, BoxDefinition, ChildBoxes, Error, FourCc, OtherBoxes, Utf8String,
    };

    use super::MediaBox;
    use crate::data_types::LanguageCode;
    use crate::elng::ExtendedLanguageBox;
    use crate::encoded;
    use crate::hdlr::HandlerBox;
    use crate::mdhd::MediaHeaderBox;
    use crate::minf::MediaInformationBox;

    /// `mdhd` of version 0, timescale 1000, duration 2000 and language `und`
    const MDHD: &[u8] =
        b"\0\0\0\x20mdhd\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x03\xe8\0\0\x07\xd0\x55\xc4\0\0";

    /// `hdlr` of a video track with an empty name
    const HDLR: &[u8] = b"\0\0\0\x21hdlr\0\0\0\0\0\0\0\0vide\0\0\0\0\0\0\0\0\0\0\0\0\0";

    /// `elng` of the tag `en`
    const ELNG: &[u8] = b"\0\0\0\x0felng\0\0\0\0en\0";

    /// `minf` of one `dinf`, which holds one `url ` of media data in the same file, and an empty `stbl`
    const MINF: &[u8] =
        b"\0\0\0\x34minf\0\0\0\x24dinf\0\0\0\x1cdref\0\0\0\0\0\0\0\x01\0\0\0\x0curl \0\0\0\x01\0\0\0\x08stbl";

    /// Box of a type `mdia` holds no field for
    const FREE: &[u8] = b"\0\0\0\x08free";

    fn media() -> MediaBox {
        MediaBox {
            mdhd: MediaHeaderBox {
                timescale: 1000,
                duration: 2000,
                language: LanguageCode::UND,
                ..MediaHeaderBox::default()
            },
            hdlr: HandlerBox {
                handler_type: FourCc::new(*b"vide"),
                name: Utf8String::default(),
            },
            elng: Some(ExtendedLanguageBox {
                extended_language: Utf8String::new(String::from("en")).unwrap(),
            }),
            minf: MediaInformationBox::decode(MINF).unwrap().0,
            other_boxes: OtherBoxes::from(ChildBoxes::decode(FREE).unwrap()),
        }
    }

    #[test]
    fn reads_its_children_and_keeps_the_child_it_has_no_field_for() {
        let bytes = [b"\0\0\0\x94mdia".as_slice(), MDHD, HDLR, FREE, MINF, ELNG].concat();

        assert_eq!(MediaBox::decode(&bytes), Ok((media(), b"".as_slice())));
    }

    #[test]
    fn writes_its_children_in_the_order_of_their_fields_then_the_child_it_has_no_field_for() {
        assert_eq!(
            encoded(&media()),
            [b"\0\0\0\x94mdia".as_slice(), MDHD, HDLR, ELNG, MINF, FREE].concat()
        );
    }

    #[test]
    fn an_extended_language_of_a_version_it_does_not_read_is_kept_among_the_other_children() {
        let version_1 = b"\0\0\0\x0felng\x01\0\0\0en\0";
        let payload = [MDHD, HDLR, MINF, version_1].concat();
        let mut expected = media();
        expected.elng = None;
        expected.other_boxes = OtherBoxes::from(ChildBoxes::decode(version_1).unwrap());

        assert_eq!(MediaBox::decode_payload(&payload), Ok(expected));
    }

    #[test]
    fn a_box_without_a_handler_is_refused() {
        assert_eq!(
            MediaBox::decode_payload(&[MDHD, MINF].concat()),
            Err(Error::missing_mandatory_box(HandlerBox::BOX_TYPE))
        );
    }
}
