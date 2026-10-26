//! [`AudioElementSelectionBox`], a set of audio elements of which one at a time is switched on, and its description

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, FullBoxFlags, FullBoxHeader, OtherBoxes, Utf8String,
};

use crate::audio_element::{
    AudioElementBox, AudioElementPosInteractivityPolar, AudioElementProminenceInteractivity,
};
use crate::full_box::read_version_zero;

/// [`AudioElementSelectionDescriptionBox`] (`aesd`), ISO/IEC 14496-12 §12.2.14
///
/// The semantic information about an audio element selection.
#[doc(alias = "aesd")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct AudioElementSelectionDescriptionBox {
    /// Flags of the box, kept as read
    pub flags: FullBoxFlags,
    /// Identifier of the audio element selection, unique among them
    pub selection_tag: Utf8String,
}

impl Default for AudioElementSelectionDescriptionBox {
    fn default() -> Self {
        Self {
            flags: FullBoxFlags::ZERO,
            selection_tag: Utf8String::default(),
        }
    }
}

impl BoxDefinition for AudioElementSelectionDescriptionBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"aesd");
}

impl BoxDecode for AudioElementSelectionDescriptionBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            flags: read_version_zero(reader)?,
            selection_tag: Utf8String::from_slice(reader.take_remainder()),
        })
    }
}

impl BoxEncode for AudioElementSelectionDescriptionBox {
    fn payload_len(&self) -> u64 {
        self.selection_tag.encoded_len().saturating_add(4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxHeader::new(0, self.flags).to_bytes())?;
        self.selection_tag.encode(writer.take_remainder())?;

        Ok(())
    }
}

/// [`AudioElementSelectionBox`] (`aesb`), ISO/IEC 14496-12 §12.2.13
///
/// A set of audio elements of a preselection of which one at a time can be
/// switched on.
///
/// An `aepp` or an `aepr` here applies to every audio element of the
/// selection, whose descriptions then hold none of their own.
///
/// The box writes `aepp`, `aepr`, `aesd` and then the audio elements,
/// whatever order they were read in: the order of ISO/IEC 14496-12 Table 2,
/// and for the audio elements, which Table 2 does not list here, right after
/// `aesd`. Children of other types are kept in
/// [`other_boxes`](Self::other_boxes), and written back after them, and so is
/// an optional child that does not read. A box of no `aesd` does not read. A
/// box of no audio element reads, but writing it is refused with
/// [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox), so a
/// box built from [`Default`] is given one before it is written.
#[doc(alias = "aesb")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct AudioElementSelectionBox {
    /// Limits within which every audio element of the selection may be placed, where they may be
    pub aepp: Option<AudioElementPosInteractivityPolar>,
    /// Limits within which the prominence of every audio element of the selection may be changed, where it may be
    pub aepr: Option<AudioElementProminenceInteractivity>,
    /// Description of the selection
    pub aesd: AudioElementSelectionDescriptionBox,
    /// Audio elements of the selection, in the order they came
    pub aelm: Vec<AudioElementBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for AudioElementSelectionBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"aesb");
}

impl BoxDecode for AudioElementSelectionBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            aepp: children.take_zero_or_one_if_decoded()?,
            aepr: children.take_zero_or_one_if_decoded()?,
            aesd: children.take_exactly_one()?,
            aelm: children.take_zero_or_more_if_decoded(),
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for AudioElementSelectionBox {
    fn payload_len(&self) -> u64 {
        [
            self.aepp.as_ref().map_or(0, BoxEncode::encoded_len),
            self.aepr.as_ref().map_or(0, BoxEncode::encoded_len),
            self.aesd.encoded_len(),
        ]
        .into_iter()
        .chain(self.aelm.iter().map(BoxEncode::encoded_len))
        .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
        .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.aelm.is_empty() {
            return Err(Error::missing_mandatory_box(AudioElementBox::BOX_TYPE));
        }

        let mut rest = writer.take_remainder();
        if let Some(aepp) = &self.aepp {
            rest = aepp.encode(rest)?;
        }
        if let Some(aepr) = &self.aepr {
            rest = aepr.encode(rest)?;
        }
        rest = self.aesd.encode(rest)?;
        for audio_element in &self.aelm {
            rest = audio_element.encode(rest)?;
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

    use isobmff_core::{
        BoxDecode, BoxDefinition, BoxEncode, ChildBoxes, Error, FullBoxFlags, OtherBoxes,
        Utf8String,
    };

    use super::{AudioElementSelectionBox, AudioElementSelectionDescriptionBox};
    use crate::audio_element::{AudioElementBox, AudioElementPosInteractivityPolar};
    use crate::encoded;

    /// `aesd` of the tag `language`
    const AESD: &[u8] = b"\0\0\0\x15aesd\0\0\0\0language\0";

    /// `aepp` of the elevation alone: -10 to 10 by 0
    const AEPP: &[u8] = b"\0\0\0\x0faepp\0\0\0\x02\xf6\x0a\0";

    /// `aelm` of an `aedb` of the tag `a`, neither toggleable nor enabled by default
    const AELM: &[u8] = b"\0\0\0\x17aelm\0\0\0\x0faedb\0\0\0\0a\0\0";

    /// `labl` holding nothing, a box this box holds no field for
    const LABL: &[u8] = b"\0\0\0\x08labl";

    fn selection() -> AudioElementSelectionBox {
        AudioElementSelectionBox {
            aepp: Some(AudioElementPosInteractivityPolar {
                min_elevation: Some(-10),
                max_elevation: Some(10),
                default_elevation: Some(0),
                ..AudioElementPosInteractivityPolar::default()
            }),
            aepr: None,
            aesd: AudioElementSelectionDescriptionBox {
                flags: FullBoxFlags::ZERO,
                selection_tag: Utf8String::new(String::from("language")).unwrap(),
            },
            aelm: vec![AudioElementBox::decode(AELM).unwrap().0; 2],
            other_boxes: OtherBoxes::from(ChildBoxes::decode(LABL).unwrap()),
        }
    }

    #[test]
    fn reads_its_children_in_any_order_and_keeps_the_child_it_has_no_field_for() {
        let bytes = [b"\0\0\0\x62aesb".as_slice(), AELM, LABL, AESD, AELM, AEPP].concat();

        assert_eq!(
            AudioElementSelectionBox::decode(&bytes),
            Ok((selection(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_aepp_then_aesd_then_its_audio_elements_then_the_child_it_has_no_field_for() {
        assert_eq!(
            encoded(&selection()),
            [b"\0\0\0\x62aesb".as_slice(), AEPP, AESD, AELM, AELM, LABL].concat()
        );
    }

    #[test]
    fn a_selection_of_no_description_is_refused() {
        assert_eq!(
            AudioElementSelectionBox::decode_payload(AELM),
            Err(Error::missing_mandatory_box(
                AudioElementSelectionDescriptionBox::BOX_TYPE
            ))
        );
    }

    #[test]
    fn a_selection_of_no_audio_element_reads_but_is_refused_when_written() {
        let (read, _) =
            AudioElementSelectionBox::decode(&[b"\0\0\0\x1daesb".as_slice(), AESD].concat())
                .unwrap();

        assert_eq!(
            read.encode(&mut [0; 29]),
            Err(Error::missing_mandatory_box(AudioElementBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_description_of_a_version_it_does_not_read_is_refused() {
        assert_eq!(
            AudioElementSelectionDescriptionBox::decode_payload(b"\x01\0\0\0a\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
