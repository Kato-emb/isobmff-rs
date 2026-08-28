//! [`MP4VisualSampleEntry`] (`mp4v`), ISO/IEC 14496-14 §6.7

use isobmff_boxes::VisualSampleEntry;
use isobmff_core::{
    AnyBox, BoxDefinition, BoxEncode, BoxType, FieldReader, FieldWriter, OtherBoxes, boxes,
};

use crate::error::Error;
use crate::esds::ESDBox;

/// Sample entry of an MPEG-4 visual track
///
/// [`MP4VisualSampleEntry`] (`mp4v`), ISO/IEC 14496-14 §6.7. The entry opens
/// with the fields of a [`VisualSampleEntry`] — §6.7.3 sets its
/// `compressorname` to 0 — and holds an [`ESDBox`]; any other box is kept as
/// it came and written back.
///
/// The payload is read by [`decode_payload`](Self::decode_payload) rather than
/// [`BoxDecode`](isobmff_core::BoxDecode), for the reason [`ESDBox`] gives.
#[doc(alias = "mp4v")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct MP4VisualSampleEntry {
    visual: VisualSampleEntry,
    es: ESDBox,
    other_boxes: OtherBoxes,
}

impl MP4VisualSampleEntry {
    /// Creates the entry from the visual fields and the descriptor box
    #[must_use]
    pub const fn new(visual: VisualSampleEntry, es: ESDBox) -> Self {
        Self {
            visual,
            es,
            other_boxes: OtherBoxes::new(),
        }
    }

    /// Returns the fields the entry opens with
    #[must_use]
    pub const fn visual(&self) -> &VisualSampleEntry {
        &self.visual
    }

    /// Returns the descriptor box, `esds`
    #[must_use]
    pub const fn es(&self) -> &ESDBox {
        &self.es
    }

    /// Returns the boxes no field claims, in the order they came
    #[must_use]
    pub fn other_boxes(&self) -> &[AnyBox] {
        self.other_boxes.as_slice()
    }

    /// Reads the entry from the payload of an `mp4v` box
    ///
    /// # Errors
    ///
    /// * [`Box`](crate::ErrorKind::Box): what [`VisualSampleEntry::decode_fields`]
    ///   reports for the fields; a child that does not frame as a box; no
    ///   `esds` among the children, or more than one.
    /// * What [`ESDBox::decode_payload`] reports.
    pub fn decode_payload(payload: &[u8]) -> Result<Self, Error> {
        let mut reader = FieldReader::new(payload);
        let visual = VisualSampleEntry::decode_fields(&mut reader)?;
        let mut es = None;
        let mut other_boxes = OtherBoxes::new();
        for child in boxes(reader.take_remainder()) {
            let child = child?;
            if child.header().box_type() == ESDBox::BOX_TYPE {
                crate::esds::decode_child(&mut es, child)?;
            } else {
                other_boxes.keep(child);
            }
        }

        Ok(Self {
            visual,
            es: es.ok_or(isobmff_core::Error::missing_mandatory_box(ESDBox::BOX_TYPE))?,
            other_boxes,
        })
    }
}

impl BoxDefinition for MP4VisualSampleEntry {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mp4v");
}

impl BoxEncode for MP4VisualSampleEntry {
    fn payload_len(&self) -> u64 {
        let others = self
            .other_boxes
            .as_slice()
            .iter()
            .fold(0_u64, |total, other| {
                total.saturating_add(other.encoded_len())
            });

        VisualSampleEntry::LEN
            .saturating_add(self.es.encoded_len())
            .saturating_add(others)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), isobmff_core::Error> {
        self.visual.encode_fields(writer)?;
        let mut rest = self.es.encode(writer.take_remainder())?;
        for other in self.other_boxes.as_slice() {
            rest = other.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_boxes::VisualSampleEntry;
    use isobmff_core::{AnyBox, BoxEncode, BoxType};

    use super::MP4VisualSampleEntry;
    use crate::error::Error;
    use crate::es_descriptor::tests::aac_descriptor;
    use crate::esds::ESDBox;

    fn entry() -> MP4VisualSampleEntry {
        MP4VisualSampleEntry::new(
            VisualSampleEntry::new(1, 640, 480),
            ESDBox::new(aac_descriptor()),
        )
    }

    fn encoded_payload(entry: &MP4VisualSampleEntry) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(entry.payload_len()).unwrap()];
        entry.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn an_entry_reads_back_as_the_value_that_wrote_it() {
        let payload = encoded_payload(&entry());

        assert_eq!(
            MP4VisualSampleEntry::decode_payload(&payload).unwrap(),
            entry()
        );
    }

    #[test]
    fn a_child_no_field_claims_is_kept_and_written_back() {
        let payload = [encoded_payload(&entry()), b"\0\0\0\x08free".to_vec()].concat();

        let entry = MP4VisualSampleEntry::decode_payload(&payload).unwrap();

        assert_eq!(
            entry.other_boxes().first().map(AnyBox::box_type),
            Some(BoxType::compact(*b"free"))
        );
        assert_eq!(encoded_payload(&entry), payload);
    }

    #[test]
    fn an_entry_holding_no_descriptor_box_is_rejected() {
        assert_eq!(
            MP4VisualSampleEntry::decode_payload(&[0; 78]),
            Err(Error::from(isobmff_core::Error::missing_mandatory_box(
                BoxType::compact(*b"esds")
            )))
        );
    }
}
