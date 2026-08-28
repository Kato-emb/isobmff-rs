//! [`MP4AudioSampleEntry`] (`mp4a`), ISO/IEC 14496-14 §6.7

use isobmff_boxes::{AudioSampleEntry, SamplingRateBox};
use isobmff_core::{
    AnyBox, BoxDefinition, BoxEncode, BoxType, ChildBoxes, FieldReader, FieldWriter, OtherBoxes,
    boxes,
};

use crate::error::Error;
use crate::esds::ESDBox;

/// Sample entry of an MPEG-4 audio track
///
/// [`MP4AudioSampleEntry`] (`mp4a`), ISO/IEC 14496-14 §6.7. The entry opens
/// with the fields of an [`AudioSampleEntry`] and holds an [`ESDBox`]; a
/// [`SamplingRateBox`] may follow for a version 1 entry, and any other box —
/// `chnl`, the DRC boxes — is kept as it came and written back.
///
/// The payload is read by [`decode_payload`](Self::decode_payload) rather than
/// [`BoxDecode`](isobmff_core::BoxDecode), for the reason [`ESDBox`] gives.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{AudioSampleEntry, SampleDescriptionBox};
/// use isobmff_core::AnyBox;
/// use isobmff_mp4::{
///     DecoderConfigDescriptor, DecoderSpecificInfo, ESDBox, ESDescriptor, MP4AudioSampleEntry,
/// };
///
/// // An AAC-LC stereo stream at 48 kHz
/// let decoder_config = DecoderConfigDescriptor::new(
///     DecoderConfigDescriptor::OBJECT_TYPE_AUDIO_ISO_14496_3,
///     DecoderConfigDescriptor::STREAM_TYPE_AUDIO,
///     6144,
///     128_000,
///     128_000,
///     Some(DecoderSpecificInfo::new(vec![0x11, 0x90]).unwrap()),
/// )
/// .unwrap();
/// let entry = MP4AudioSampleEntry::new(
///     AudioSampleEntry::new(1, 2, 48_000),
///     ESDBox::new(ESDescriptor::for_mp4_file(decoder_config)),
///     None,
/// );
///
/// // The entry goes into a `stsd` as any other, and comes back out typed
/// let description = SampleDescriptionBox::new(vec![AnyBox::from(entry.clone())]);
/// let found = description.entries()[0].downcast_ref::<MP4AudioSampleEntry>().unwrap();
/// assert_eq!(found, &entry);
/// ```
#[doc(alias = "mp4a")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct MP4AudioSampleEntry {
    audio: AudioSampleEntry,
    es: ESDBox,
    sampling_rate: Option<SamplingRateBox>,
    other_boxes: OtherBoxes,
}

impl MP4AudioSampleEntry {
    /// Creates the entry from the audio fields, the descriptor box, and the
    /// sampling rate box a version 1 entry states its rate in
    #[must_use]
    pub const fn new(
        audio: AudioSampleEntry,
        es: ESDBox,
        sampling_rate: Option<SamplingRateBox>,
    ) -> Self {
        Self {
            audio,
            es,
            sampling_rate,
            other_boxes: OtherBoxes::new(),
        }
    }

    /// Returns the fields the entry opens with
    #[must_use]
    pub const fn audio(&self) -> &AudioSampleEntry {
        &self.audio
    }

    /// Returns the descriptor box, `esds`
    #[must_use]
    pub const fn es(&self) -> &ESDBox {
        &self.es
    }

    /// Returns the sampling rate box, `srat`, when the entry holds one
    #[must_use]
    pub const fn sampling_rate(&self) -> Option<&SamplingRateBox> {
        self.sampling_rate.as_ref()
    }

    /// Returns the boxes no field claims, in the order they came
    #[must_use]
    pub fn other_boxes(&self) -> &[AnyBox] {
        self.other_boxes.as_slice()
    }

    /// Reads the entry from the payload of an `mp4a` box
    ///
    /// A `stsd` entry arrives as an [`AnyBox`]; one whose
    /// [`box_type`](AnyBox::box_type) is [`BOX_TYPE`](Self::BOX_TYPE) hands
    /// its [`raw_payload`](AnyBox::raw_payload) here.
    ///
    /// # Errors
    ///
    /// * [`Box`](crate::ErrorKind::Box): what [`AudioSampleEntry::decode_fields`]
    ///   reports for the fields; a child that does not frame as a box; no
    ///   `esds` among the children, or more than one `esds` or `srat`; what
    ///   [`SamplingRateBox`] reports.
    /// * What [`ESDBox::decode_payload`] reports.
    pub fn decode_payload(payload: &[u8]) -> Result<Self, Error> {
        let mut reader = FieldReader::new(payload);
        let audio = AudioSampleEntry::decode_fields(&mut reader)?;

        let mut es = None;
        let mut sampling_rate_boxes = ChildBoxes::new();
        let mut other_boxes = OtherBoxes::new();
        for child in boxes(reader.take_remainder()) {
            let child = child?;
            let box_type = child.header().box_type();
            if box_type == ESDBox::BOX_TYPE {
                crate::esds::decode_child(&mut es, child)?;
            } else if box_type == SamplingRateBox::BOX_TYPE {
                sampling_rate_boxes.push(child);
            } else {
                other_boxes.keep(child);
            }
        }

        Ok(Self {
            audio,
            es: es.ok_or(isobmff_core::Error::missing_mandatory_box(ESDBox::BOX_TYPE))?,
            sampling_rate: sampling_rate_boxes.zero_or_one()?,
            other_boxes,
        })
    }
}

impl BoxDefinition for MP4AudioSampleEntry {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mp4a");
}

impl BoxEncode for MP4AudioSampleEntry {
    fn payload_len(&self) -> u64 {
        let sampling_rate = self
            .sampling_rate
            .as_ref()
            .map_or(0, BoxEncode::encoded_len);
        let others = self
            .other_boxes
            .as_slice()
            .iter()
            .fold(0_u64, |total, other| {
                total.saturating_add(other.encoded_len())
            });

        AudioSampleEntry::LEN
            .saturating_add(self.es.encoded_len())
            .saturating_add(sampling_rate)
            .saturating_add(others)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), isobmff_core::Error> {
        self.audio.encode_fields(writer)?;
        let mut rest = self.es.encode(writer.take_remainder())?;
        if let Some(sampling_rate) = &self.sampling_rate {
            rest = sampling_rate.encode(rest)?;
        }
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

    use isobmff_boxes::{AudioSampleEntry, SamplingRateBox};
    use isobmff_core::{AnyBox, BoxEncode, BoxType, FourCC};

    use super::MP4AudioSampleEntry;
    use crate::error::Error;
    use crate::es_descriptor::tests::aac_descriptor;
    use crate::esds::ESDBox;

    fn entry() -> MP4AudioSampleEntry {
        MP4AudioSampleEntry::new(
            AudioSampleEntry::new(1, 2, 48_000),
            ESDBox::new(aac_descriptor()),
            None,
        )
    }

    fn encoded_payload(entry: &MP4AudioSampleEntry) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(entry.payload_len()).unwrap()];
        entry.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn an_entry_reads_back_as_the_value_that_wrote_it() {
        let payload = encoded_payload(&entry());

        assert_eq!(
            MP4AudioSampleEntry::decode_payload(&payload).unwrap(),
            entry()
        );
    }

    #[test]
    fn a_version_1_entry_carries_its_rate_in_a_sampling_rate_box() {
        let entry = MP4AudioSampleEntry::new(
            AudioSampleEntry::new_v1(1, 2),
            ESDBox::new(aac_descriptor()),
            Some(SamplingRateBox::new(96_000)),
        );

        let read_back = MP4AudioSampleEntry::decode_payload(&encoded_payload(&entry)).unwrap();

        assert_eq!(read_back, entry);
        assert_eq!(
            read_back
                .sampling_rate()
                .map(SamplingRateBox::sampling_rate),
            Some(96_000)
        );
    }

    #[test]
    fn a_child_no_field_claims_is_kept_and_written_back() {
        let payload = [encoded_payload(&entry()), b"\0\0\0\x08free".to_vec()].concat();

        let entry = MP4AudioSampleEntry::decode_payload(&payload).unwrap();

        assert_eq!(
            entry.other_boxes().first().map(AnyBox::box_type),
            Some(BoxType::compact(*b"free"))
        );
        assert_eq!(encoded_payload(&entry), payload);
    }

    #[test]
    fn an_entry_holding_no_descriptor_box_is_rejected() {
        let payload = [vec![0; 28], b"\0\0\0\x08free".to_vec()].concat();

        assert_eq!(
            MP4AudioSampleEntry::decode_payload(&payload),
            Err(Error::from(isobmff_core::Error::missing_mandatory_box(
                BoxType::compact(*b"esds")
            )))
        );
    }

    #[test]
    fn a_failure_inside_the_descriptor_box_names_it_on_the_path() {
        let payload = [vec![0; 28], b"\0\0\0\x0aesds\0\0".to_vec()].concat();

        let failure = MP4AudioSampleEntry::decode_payload(&payload).unwrap_err();

        assert_eq!(
            failure
                .box_error()
                .map(|error| error.containers().collect::<Vec<_>>()),
            Some(vec![FourCC::new(*b"esds")])
        );
    }

    #[test]
    fn an_entry_holding_two_descriptor_boxes_is_rejected() {
        let entry = entry();
        let mut esds = vec![0; usize::try_from(entry.es().encoded_len()).unwrap()];
        entry.es().encode(&mut esds).unwrap();
        let payload = [encoded_payload(&entry), esds].concat();

        assert_eq!(
            MP4AudioSampleEntry::decode_payload(&payload),
            Err(Error::from(isobmff_core::Error::duplicate_box(
                BoxType::compact(*b"esds")
            )))
        );
    }
}
