//! [`MpegSampleEntry`] (`mp4s`), ISO/IEC 14496-14 §6.7

use isobmff_core::{
    AnyBox, BoxDefinition, BoxEncode, BoxType, FieldReader, FieldWriter, OtherBoxes,
};

use crate::error::Error;
use crate::esds::ESDBox;

/// Length of the `SampleEntry` fields the entry opens with
const SAMPLE_ENTRY_LEN: u64 = 8;

/// Sample entry of an MPEG-4 stream of any other type — scene description,
/// object descriptor, clock reference
///
/// [`MpegSampleEntry`] (`mp4s`), ISO/IEC 14496-14 §6.7. The entry opens with
/// the fields of the plain `SampleEntry` of ISO/IEC 14496-12 §8.5.2 — six
/// reserved bytes and `data_reference_index` — and holds an [`ESDBox`]; any
/// other box is kept as it came and written back.
///
/// The payload is read by [`decode_payload`](Self::decode_payload) rather than
/// [`BoxDecode`](isobmff_core::BoxDecode), for the reason [`ESDBox`] gives.
#[doc(alias = "mp4s")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct MpegSampleEntry {
    data_reference_index: u16,
    es: ESDBox,
    other_boxes: OtherBoxes,
}

impl MpegSampleEntry {
    /// Creates the entry from the data reference its samples are read through
    /// and the descriptor box
    #[must_use]
    pub const fn new(data_reference_index: u16, es: ESDBox) -> Self {
        Self {
            data_reference_index,
            es,
            other_boxes: OtherBoxes::new(),
        }
    }

    /// Returns the index of the data reference the samples are read through
    #[must_use]
    pub const fn data_reference_index(&self) -> u16 {
        self.data_reference_index
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

    /// Reads the entry from the payload of an `mp4s` box
    ///
    /// # Errors
    ///
    /// * [`Box`](crate::ErrorKind::Box): the payload ends before the fields do;
    ///   a child that does not frame as a box; no `esds` among the children,
    ///   or more than one.
    /// * What [`ESDBox::decode_payload`] reports.
    pub fn decode_payload(payload: &[u8]) -> Result<Self, Error> {
        let mut reader = FieldReader::new(payload);
        let _reserved = reader.read_bytes::<6>()?;
        let data_reference_index = reader.read_u16()?;
        let (es, other_boxes) = crate::esds::sort_children(reader.take_remainder())?;

        Ok(Self {
            data_reference_index,
            es,
            other_boxes,
        })
    }
}

impl BoxDefinition for MpegSampleEntry {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mp4s");
}

impl BoxEncode for MpegSampleEntry {
    fn payload_len(&self) -> u64 {
        let others = self
            .other_boxes
            .as_slice()
            .iter()
            .fold(0_u64, |total, other| {
                total.saturating_add(other.encoded_len())
            });

        SAMPLE_ENTRY_LEN
            .saturating_add(self.es.encoded_len())
            .saturating_add(others)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), isobmff_core::Error> {
        writer.write_bytes(&[0; 6])?;
        writer.write_u16(self.data_reference_index)?;
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

    use isobmff_core::{AnyBox, BoxEncode, BoxType};

    use super::MpegSampleEntry;
    use crate::error::Error;
    use crate::es_descriptor::tests::aac_descriptor;
    use crate::esds::ESDBox;

    fn entry() -> MpegSampleEntry {
        MpegSampleEntry::new(1, ESDBox::new(aac_descriptor()))
    }

    fn encoded_payload(entry: &MpegSampleEntry) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(entry.payload_len()).unwrap()];
        entry.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn an_entry_reads_back_as_the_value_that_wrote_it() {
        let payload = encoded_payload(&entry());

        assert_eq!(payload.get(..8), Some(b"\0\0\0\0\0\0\0\x01".as_slice()));
        assert_eq!(MpegSampleEntry::decode_payload(&payload).unwrap(), entry());
    }

    #[test]
    fn a_child_no_field_claims_is_kept_and_written_back() {
        let payload = [encoded_payload(&entry()), b"\0\0\0\x08free".to_vec()].concat();

        let entry = MpegSampleEntry::decode_payload(&payload).unwrap();

        assert_eq!(
            entry.other_boxes().first().map(AnyBox::box_type),
            Some(BoxType::compact(*b"free"))
        );
        assert_eq!(encoded_payload(&entry), payload);
    }

    #[test]
    fn an_entry_holding_no_descriptor_box_is_rejected() {
        assert_eq!(
            MpegSampleEntry::decode_payload(&[0; 8]),
            Err(Error::from(isobmff_core::Error::missing_mandatory_box(
                BoxType::compact(*b"esds")
            )))
        );
    }

    #[test]
    fn a_payload_ending_before_the_fields_is_rejected_as_truncated() {
        assert_eq!(
            MpegSampleEntry::decode_payload(&[0; 7]),
            Err(Error::from(isobmff_core::Error::truncated_payload(8, 7)))
        );
    }
}
