//! [`MP4AudioSampleEntry`] (`mp4a`), ISO/IEC 14496-14 §6.7

use isobmff_boxes::{AudioSampleEntry, SamplingRateBox};
use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, Boxes, ChildBoxes, FieldReader,
    FieldWriter, OtherBoxes,
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
/// An entry read from a file lies in its `stsd` as the bytes it came as.
/// [`SampleDescriptionBox::audio_entry`](isobmff_boxes::SampleDescriptionBox::audio_entry)
/// reads it against the version of the `stsd`, refusing a QuickTime sound
/// description of version 1; [`decode_payload`](BoxDecode::decode_payload)
/// reads the payload alone, and reads such a description as an
/// `AudioSampleEntryV1`.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{AudioSampleEntry, SampleDescriptionBox};
/// use isobmff_core::{AnyBox, U16F16};
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
///     AudioSampleEntry::new(1, U16F16::from_integer(48_000)),
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
}

impl BoxDefinition for MP4AudioSampleEntry {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mp4a");
}

impl BoxDecode for MP4AudioSampleEntry {
    type Error = Error;

    /// # Errors
    ///
    /// * [`Box`](crate::ErrorKind::Box): what [`AudioSampleEntry::decode_fields`]
    ///   reports for the fields; a child that does not frame as a box; no
    ///   `esds` among the children, or more than one `esds` or `srat`; what
    ///   [`SamplingRateBox`] reports, with `srat` on the
    ///   [`containers`](isobmff_core::Error::containers) path.
    /// * What the [`BoxDecode`] of [`ESDBox`] reports, with `esds` on the
    ///   [`containers`](isobmff_core::Error::containers) path of a box failure.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let audio = AudioSampleEntry::decode_fields(reader)?;

        let mut children: ChildBoxes<'_> =
            Boxes::new(reader.take_remainder()).collect::<Result<_, _>>()?;

        Ok(Self {
            audio,
            es: children.take_exactly_one()?,
            sampling_rate: children.take_zero_or_one()?,
            other_boxes: OtherBoxes::from(children),
        })
    }
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

    use isobmff_boxes::{AudioSampleEntry, SampleDescriptionBox, SamplingRateBox};
    use isobmff_core::{AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, FourCc, U16F16};

    use super::MP4AudioSampleEntry;
    use crate::error::Error;
    use crate::es_descriptor::tests::aac_descriptor;
    use crate::esds::ESDBox;

    fn entry() -> MP4AudioSampleEntry {
        MP4AudioSampleEntry::new(
            AudioSampleEntry::new(1, U16F16::from_integer(48_000)),
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
            Some(vec![FourCc::new(*b"esds")])
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

    #[test]
    fn a_quicktime_version_1_entry_in_a_description_of_version_0_is_refused() {
        let mut payload = encoded_payload(&entry());
        *payload.get_mut(9).unwrap() = 1;
        let (fields, boxes) = payload.split_at(usize::try_from(AudioSampleEntry::LEN).unwrap());
        let samples_per_packet_to_bytes_per_sample =
            [0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2];
        let quicktime_version_1 = AnyBox::from_raw_bytes(
            MP4AudioSampleEntry::BOX_TYPE,
            [fields, &samples_per_packet_to_bytes_per_sample, boxes].concat(),
        );

        let description = SampleDescriptionBox::new(vec![quicktime_version_1]);

        assert_eq!(
            description.audio_entry::<MP4AudioSampleEntry>(0),
            Some(Err(Error::from(
                isobmff_core::Error::unsupported_version(1)
                    .in_container(MP4AudioSampleEntry::BOX_TYPE)
            )))
        );
    }

    #[test]
    fn an_entry_of_either_version_in_a_description_of_version_1_reads() {
        let version_1 = MP4AudioSampleEntry::new(
            AudioSampleEntry::new_v1(1, 2),
            ESDBox::new(aac_descriptor()),
            Some(SamplingRateBox::new(96_000)),
        );
        let description = SampleDescriptionBox::new_v1(vec![
            AnyBox::from(entry()),
            AnyBox::from_raw_bytes(MP4AudioSampleEntry::BOX_TYPE, encoded_payload(&version_1)),
        ]);

        let read = [0, 1].map(|index| description.audio_entry::<MP4AudioSampleEntry>(index));

        assert_eq!(read, [Some(Ok(entry())), Some(Ok(version_1))]);
    }

    #[test]
    fn an_entry_of_another_type_is_refused_naming_its_type() {
        let avc1 = BoxType::compact(*b"avc1");
        let description = SampleDescriptionBox::new(vec![AnyBox::from_raw_bytes(
            avc1,
            encoded_payload(&entry()),
        )]);

        assert_eq!(
            description.audio_entry::<MP4AudioSampleEntry>(0),
            Some(Err(Error::from(
                isobmff_core::Error::box_type_mismatch(MP4AudioSampleEntry::BOX_TYPE, avc1)
                    .in_container(avc1)
            )))
        );
    }

    #[test]
    fn an_index_past_the_entries_reads_nothing() {
        let description = SampleDescriptionBox::new(vec![AnyBox::from(entry())]);

        assert_eq!(description.audio_entry::<MP4AudioSampleEntry>(1), None);
    }
}
