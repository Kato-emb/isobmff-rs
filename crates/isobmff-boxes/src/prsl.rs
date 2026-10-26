//! [`PreselectionGroupBox`], a preselection of tracks and what it offers

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, FullBoxFlags, FullBoxHeader, OtherBoxes, Utf8String,
};

use crate::ardi::AudioRenderingIndicationBox;
use crate::audio_element::AudioElementBox;
use crate::audio_element_selection::AudioElementSelectionBox;
use crate::chnl::ChannelLayout;
use crate::elng::ExtendedLanguageBox;
use crate::full_box::read_version_zero;
use crate::grpl::{entities_len, read_entities, write_entities};
use crate::infe::{read_string, write_string};

/// Bit of `flags` that says `preselection_tag` is there
const PRESELECTION_TAG_PRESENT: u32 = 0x00_1000;
/// Bit of `flags` that says `selection_priority` is there
const SELECTION_PRIORITY_PRESENT: u32 = 0x00_2000;
/// Bit of `flags` that says `interleaving_tag` is there
const INTERLEAVING_TAG_PRESENT: u32 = 0x00_4000;

/// [`PreselectionGroupBox`] (`prsl`), ISO/IEC 14496-12 §8.15.4.1
///
/// A preselection: the tracks that together give one experience, in the
/// order they are processed, and boxes that describe the experience, such as
/// its language or its audio elements. It is the entity group of the type
/// `prsl`, held in the `GroupsListBox` of a file-level `MetaBox`.
///
/// The `flags` of the box are whether each optional field is there. The box
/// writes `elng`, `chnl`, `ardi`, the `aelm`s and the `aesb`s in this order,
/// whatever order they were read in: the order of ISO/IEC 14496-12 Table 2.
/// Children of other types are kept in [`other_boxes`](Self::other_boxes),
/// and written back after them, and so is a child that does not read; an
/// `elng`, a `chnl` or an `ardi` that does not read reads back [`None`].
#[doc(alias = "prsl")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct PreselectionGroupBox {
    /// Identifier of the preselection, which no other entity group, item or track shares
    pub group_id: u32,
    /// `track_ID`, or `group_id` of another entity group, of each entity, in the order they are processed
    pub entity_ids: Vec<u32>,
    /// Codec specific value that identifies the preselection to a decoder
    pub preselection_tag: Option<Utf8String>,
    /// Priority of the preselection where nothing else tells them apart, a lower value a higher priority
    pub selection_priority: Option<u8>,
    /// Process that interleaves the samples of the entities, none where empty
    pub interleaving_tag: Option<Utf8String>,
    /// Language of the preselection
    pub elng: Option<ExtendedLanguageBox>,
    /// Channel layout of the preselection
    pub chnl: Option<ChannelLayout>,
    /// Reproduction channel layout the preselection prefers
    pub ardi: Option<AudioRenderingIndicationBox>,
    /// Audio elements of the preselection
    pub aelm: Vec<AudioElementBox>,
    /// Sets of audio elements of which one at a time is switched on
    pub aesb: Vec<AudioElementSelectionBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for PreselectionGroupBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"prsl");
}

impl BoxDecode for PreselectionGroupBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let flags = read_version_zero(reader)?.bits();
        let (group_id, entity_ids) = read_entities(reader)?;
        let preselection_tag = (flags & PRESELECTION_TAG_PRESENT != 0)
            .then(|| read_string(reader))
            .transpose()?;
        let selection_priority = (flags & SELECTION_PRIORITY_PRESENT != 0)
            .then(|| reader.read_bytes::<1>().map(|[priority]| *priority))
            .transpose()?;
        let interleaving_tag = (flags & INTERLEAVING_TAG_PRESENT != 0)
            .then(|| read_string(reader))
            .transpose()?;
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            group_id,
            entity_ids,
            preselection_tag,
            selection_priority,
            interleaving_tag,
            elng: children.take_zero_or_one_if_decoded()?,
            chnl: children.take_zero_or_one_if_decoded()?,
            ardi: children.take_zero_or_one_if_decoded()?,
            aelm: children.take_zero_or_more_if_decoded(),
            aesb: children.take_zero_or_more_if_decoded(),
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for PreselectionGroupBox {
    fn payload_len(&self) -> u64 {
        [
            4,
            entities_len(&self.entity_ids),
            self.preselection_tag
                .as_ref()
                .map_or(0, Utf8String::encoded_len),
            self.selection_priority.map_or(0, |_| 1),
            self.interleaving_tag
                .as_ref()
                .map_or(0, Utf8String::encoded_len),
            self.elng.as_ref().map_or(0, BoxEncode::encoded_len),
            self.chnl.as_ref().map_or(0, BoxEncode::encoded_len),
            self.ardi.as_ref().map_or(0, BoxEncode::encoded_len),
        ]
        .into_iter()
        .chain(self.aelm.iter().map(BoxEncode::encoded_len))
        .chain(self.aesb.iter().map(BoxEncode::encoded_len))
        .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
        .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let flags = [
            (self.preselection_tag.is_some(), PRESELECTION_TAG_PRESENT),
            (
                self.selection_priority.is_some(),
                SELECTION_PRIORITY_PRESENT,
            ),
            (self.interleaving_tag.is_some(), INTERLEAVING_TAG_PRESENT),
        ]
        .into_iter()
        .filter(|(present, _)| *present)
        .fold(0, |flags, (_, bit)| flags | bit);
        let flags = FullBoxFlags::new(flags).unwrap_or(FullBoxFlags::ZERO);

        writer.write_bytes(&FullBoxHeader::new(0, flags).to_bytes())?;
        write_entities(writer, self.group_id, &self.entity_ids)?;
        if let Some(preselection_tag) = &self.preselection_tag {
            write_string(writer, preselection_tag)?;
        }
        if let Some(selection_priority) = self.selection_priority {
            writer.write_bytes(&[selection_priority])?;
        }
        if let Some(interleaving_tag) = &self.interleaving_tag {
            write_string(writer, interleaving_tag)?;
        }
        let mut rest = writer.take_remainder();
        if let Some(elng) = &self.elng {
            rest = elng.encode(rest)?;
        }
        if let Some(chnl) = &self.chnl {
            rest = chnl.encode(rest)?;
        }
        if let Some(ardi) = &self.ardi {
            rest = ardi.encode(rest)?;
        }
        for audio_element in &self.aelm {
            rest = audio_element.encode(rest)?;
        }
        for selection in &self.aesb {
            rest = selection.encode(rest)?;
        }
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec;

    use isobmff_core::{BoxDecode, ChildBoxes, Error, OtherBoxes, Utf8String};

    use super::PreselectionGroupBox;
    use crate::ardi::AudioRenderingIndicationBox;
    use crate::chnl::ChannelLayout;
    use crate::elng::ExtendedLanguageBox;
    use crate::encoded;

    /// `elng` of `en`
    const ELNG: &[u8] = b"\0\0\0\x0felng\0\0\0\0en\0";

    /// `chnl` of version 0 that announces neither channels nor objects
    const CHNL: &[u8] = b"\0\0\0\x0dchnl\0\0\0\0\0";

    /// `ardi` of the stereo layout
    const ARDI: &[u8] = b"\0\0\0\x0dardi\0\0\0\0\x01";

    /// `labl` holding nothing, a box this box holds no field for
    const LABL: &[u8] = b"\0\0\0\x08labl";

    fn string(text: &str) -> Utf8String {
        Utf8String::new(String::from(text)).unwrap()
    }

    fn preselection() -> PreselectionGroupBox {
        PreselectionGroupBox {
            group_id: 12,
            entity_ids: vec![1, 2],
            preselection_tag: Some(string("p1")),
            selection_priority: Some(3),
            interleaving_tag: Some(string("")),
            elng: Some(ExtendedLanguageBox {
                extended_language: string("en"),
            }),
            chnl: Some(ChannelLayout::Version0 {
                channel_structure: None,
                object_count: None,
            }),
            ardi: Some(AudioRenderingIndicationBox {
                audio_rendering_indication: 1,
            }),
            other_boxes: OtherBoxes::from(ChildBoxes::decode(LABL).unwrap()),
            ..PreselectionGroupBox::default()
        }
    }

    /// Fields of [`preselection`]: flags 0x007000, group 12, entities 1 and 2, `p1`, priority 3 and an empty interleaving tag
    const FIELDS: &[u8] = b"\0\0\x70\0\0\0\0\x0c\0\0\0\x02\0\0\0\x01\0\0\0\x02p1\0\x03\0";

    #[test]
    fn reads_its_optional_fields_as_its_flags_say_and_its_children_in_any_order() {
        let bytes = [b"\0\0\0\x52prsl".as_slice(), FIELDS, LABL, ARDI, CHNL, ELNG].concat();

        assert_eq!(
            PreselectionGroupBox::decode(&bytes),
            Ok((preselection(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_flags_of_the_optional_fields_it_holds_then_its_children_in_the_order_of_table_2()
    {
        assert_eq!(
            encoded(&preselection()),
            [b"\0\0\0\x52prsl".as_slice(), FIELDS, ELNG, CHNL, ARDI, LABL].concat()
        );
    }

    #[test]
    fn a_box_of_no_optional_field_reads_none_of_them() {
        let expected = PreselectionGroupBox {
            group_id: 12,
            entity_ids: vec![1],
            ..PreselectionGroupBox::default()
        };

        assert_eq!(
            PreselectionGroupBox::decode_payload(b"\0\0\0\0\0\0\0\x0c\0\0\0\x01\0\0\0\x01"),
            Ok(expected)
        );
    }

    #[test]
    fn a_payload_ending_inside_a_field_its_flags_say_is_there_is_refused() {
        assert_eq!(
            PreselectionGroupBox::decode_payload(b"\0\0\x20\0\0\0\0\x0c\0\0\0\0"),
            Err(Error::truncated_payload(13, 12))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            PreselectionGroupBox::decode_payload(b"\x01\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
