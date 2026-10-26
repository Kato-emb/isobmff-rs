//! [`MediaInformationBox`], the characteristic information of the media in a track

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::dinf::DataInformationBox;
use crate::media_information_header::MediaInformationHeader;
use crate::stbl::SampleTableBox;

/// [`MediaInformationBox`] (`minf`), ISO/IEC 14496-12 §8.4.4
///
/// Holds the boxes that declare the characteristic information of the media
/// in a track.
///
/// `media_information_header` is [`None`] where the specific media header the
/// box holds is not one ISO/IEC 14496-12 defines, such as one a derived
/// specification defines: that header is kept in
/// [`other_boxes`](Self::other_boxes). Children of other types are kept
/// there too.
///
/// The box writes the specific media header, `dinf` and `stbl` in this order,
/// whatever order they were read in: the order of ISO/IEC 14496-12 §7.3.4.
/// The children kept in [`other_boxes`](Self::other_boxes) are written back
/// after them.
#[doc(alias = "minf")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct MediaInformationBox {
    /// Specific media header of the track
    pub media_information_header: Option<MediaInformationHeader>,
    /// Where the media data of the track lies
    pub dinf: DataInformationBox,
    /// Time and data indexing of the samples of the track
    pub stbl: SampleTableBox,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for MediaInformationBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"minf");
}

impl BoxDecode for MediaInformationBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            media_information_header: children.take_zero_or_one_variant()?,
            dinf: children.take_exactly_one()?,
            stbl: children.take_exactly_one()?,
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for MediaInformationBox {
    fn payload_len(&self) -> u64 {
        let header_len = self
            .media_information_header
            .as_ref()
            .map_or(0, |header| match header {
                MediaInformationHeader::Video(video) => video.encoded_len(),
                MediaInformationHeader::Sound(sound) => sound.encoded_len(),
                MediaInformationHeader::Hint(hint) => hint.encoded_len(),
                MediaInformationHeader::Subtitle(subtitle) => subtitle.encoded_len(),
                MediaInformationHeader::Null(null) => null.encoded_len(),
                MediaInformationHeader::VolumetricVisual(volumetric_visual) => {
                    volumetric_visual.encoded_len()
                }
            });

        [header_len, self.dinf.encoded_len(), self.stbl.encoded_len()]
            .into_iter()
            .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
            .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = writer.take_remainder();
        if let Some(header) = &self.media_information_header {
            rest = match header {
                MediaInformationHeader::Video(video) => video.encode(rest)?,
                MediaInformationHeader::Sound(sound) => sound.encode(rest)?,
                MediaInformationHeader::Hint(hint) => hint.encode(rest)?,
                MediaInformationHeader::Subtitle(subtitle) => subtitle.encode(rest)?,
                MediaInformationHeader::Null(null) => null.encode(rest)?,
                MediaInformationHeader::VolumetricVisual(volumetric_visual) => {
                    volumetric_visual.encode(rest)?
                }
            };
        }
        rest = self.dinf.encode(rest)?;
        rest = self.stbl.encode(rest)?;
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxDefinition, BoxType, ChildBoxes, Error, OtherBoxes};

    use super::MediaInformationBox;
    use crate::data_entry::{DataEntry, DataEntryUrlBox};
    use crate::dinf::DataInformationBox;
    use crate::dref::DataReferenceBox;
    use crate::encoded;
    use crate::media_information_header::{MediaInformationHeader, SoundMediaHeaderBox};
    use crate::stbl::SampleTableBox;

    /// `smhd` of balance 0
    const SMHD: &[u8] = b"\0\0\0\x10smhd\0\0\0\0\0\0\0\0";

    /// `dinf` of one `url ` of media data in the same file
    const DINF: &[u8] = b"\0\0\0\x24dinf\0\0\0\x1cdref\0\0\0\0\0\0\0\x01\0\0\0\x0curl \0\0\0\x01";

    /// `stbl` holding an `stsd` of no entry alone, of a track that references no data
    const STBL: &[u8] = b"\0\0\0\x18stbl\0\0\0\x10stsd\0\0\0\0\0\0\0\0";

    /// `free`, a box `minf` holds no field for
    const FREE: &[u8] = b"\0\0\0\x08free";

    fn data_information() -> DataInformationBox {
        DataInformationBox {
            dref: DataReferenceBox {
                data_entries: vec![DataEntry::Url(DataEntryUrlBox::default())],
            },
            other_boxes: OtherBoxes::new(),
        }
    }

    fn media_information() -> MediaInformationBox {
        MediaInformationBox {
            media_information_header: Some(MediaInformationHeader::Sound(
                SoundMediaHeaderBox::default(),
            )),
            dinf: data_information(),
            stbl: SampleTableBox::default(),
            other_boxes: OtherBoxes::from(ChildBoxes::decode(FREE).unwrap()),
        }
    }

    #[test]
    fn reads_its_children_and_keeps_the_child_it_has_no_field_for() {
        let bytes = [b"\0\0\0\x5cminf".as_slice(), FREE, STBL, DINF, SMHD].concat();

        assert_eq!(
            MediaInformationBox::decode(&bytes),
            Ok((media_information(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_its_children_in_the_order_of_their_fields_then_the_child_it_has_no_field_for() {
        assert_eq!(
            encoded(&media_information()),
            [b"\0\0\0\x5cminf".as_slice(), SMHD, DINF, STBL, FREE].concat()
        );
    }

    #[test]
    fn a_media_header_a_derived_specification_defines_is_kept_among_the_other_children() {
        let gmhd = b"\0\0\0\x08gmhd";
        let payload = [gmhd.as_slice(), DINF, STBL].concat();

        assert_eq!(
            MediaInformationBox::decode_payload(&payload),
            Ok(MediaInformationBox {
                media_information_header: None,
                dinf: data_information(),
                stbl: SampleTableBox::default(),
                other_boxes: OtherBoxes::from(ChildBoxes::decode(gmhd).unwrap()),
            })
        );
    }

    #[test]
    fn a_media_header_that_does_not_read_is_refused() {
        let payload = [b"\0\0\0\x0csmhd\x01\0\0\0".as_slice(), DINF].concat();

        assert_eq!(
            MediaInformationBox::decode_payload(&payload),
            Err(Error::unsupported_version(1).in_container(BoxType::compact(*b"smhd")))
        );
    }

    #[test]
    fn a_box_without_a_data_information_is_refused() {
        assert_eq!(
            MediaInformationBox::decode_payload(SMHD),
            Err(Error::missing_mandatory_box(DataInformationBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_box_without_a_sample_table_is_refused() {
        let payload = [SMHD, DINF].concat();

        assert_eq!(
            MediaInformationBox::decode_payload(&payload),
            Err(Error::missing_mandatory_box(SampleTableBox::BOX_TYPE))
        );
    }
}
