//! [`AVCSampleEntry`] (`avc1`, `avc3`), ISO/IEC 14496-15 §5.4.2

use isobmff_boxes::{BitRateBox, VisualSampleEntry};
use isobmff_core::{
    AnyBox, BoxDefinition, BoxEncode, BoxFormat, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes, boxes,
};

use crate::avcc::AVCConfigurationBox;

const AVC1: BoxType = BoxType::compact(*b"avc1");
const AVC3: BoxType = BoxType::compact(*b"avc3");

/// Code an AVC sample entry is named by, stating where its parameter sets lie
///
/// ISO/IEC 14496-15 §5.4.2 lays one class over two codes: under `avc1` the
/// parameter sets lie in the sample entry alone, under `avc3` they may also lie
/// in the samples, as NAL units of the stream.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AVCSampleEntryType {
    /// `avc1`: parameter sets in the sample entry only
    Avc1,
    /// `avc3`: parameter sets in the sample entry or in the samples
    Avc3,
}

impl AVCSampleEntryType {
    /// Returns the code `box_type` names, when it names one of the two
    #[must_use]
    pub fn from_box_type(box_type: BoxType) -> Option<Self> {
        if box_type == AVC1 {
            Some(Self::Avc1)
        } else if box_type == AVC3 {
            Some(Self::Avc3)
        } else {
            None
        }
    }

    /// Returns the box type the code is written as
    #[must_use]
    pub const fn box_type(self) -> BoxType {
        match self {
            Self::Avc1 => AVC1,
            Self::Avc3 => AVC3,
        }
    }
}

/// Sample entry of an AVC video track
///
/// [`AVCSampleEntry`] (`avc1` or `avc3`), ISO/IEC 14496-15 §5.4.2. The entry
/// opens with the fields of a [`VisualSampleEntry`] and holds an
/// [`AVCConfigurationBox`]; a [`BitRateBox`] may follow, and any other box —
/// `m4ds`, `pasp`, `colr`, the boxes a later specification adds — is kept as it
/// came and written back.
///
/// # Examples
///
/// ```
/// use isobmff_avc::{
///     AVCConfigurationBox, AVCDecoderConfigurationRecord, AVCSampleEntry, AVCSampleEntryType,
///     LengthSizeMinusOne,
/// };
/// use isobmff_boxes::{SampleDescriptionBox, VisualSampleEntry};
/// use isobmff_core::AnyBox;
///
/// // A 1920 by 1080 stream whose parameter sets travel in-band
/// let record = AVCDecoderConfigurationRecord::from_parameter_sets(
///     LengthSizeMinusOne::FOUR_BYTES,
///     vec![vec![0x67, 0x64, 0x00, 0x28]],
///     vec![vec![0x68, 0xce, 0x3c, 0x80]],
///     None,
/// )
/// .unwrap();
/// let entry = AVCSampleEntry::new(
///     AVCSampleEntryType::Avc3,
///     VisualSampleEntry::new(1, 1920, 1080),
///     AVCConfigurationBox::new(record),
///     None,
/// );
///
/// // The entry goes into a `stsd` as any other, and comes back out typed
/// let description = SampleDescriptionBox::new(vec![AnyBox::from(entry.clone())]);
/// let found = description.entries()[0].downcast_ref::<AVCSampleEntry>().unwrap();
/// assert_eq!(found, &entry);
/// assert_eq!(found.entry_type(), AVCSampleEntryType::Avc3);
/// ```
#[doc(alias = "avc1")]
#[doc(alias = "avc3")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct AVCSampleEntry {
    entry_type: AVCSampleEntryType,
    visual: VisualSampleEntry,
    config: AVCConfigurationBox,
    bit_rate: Option<BitRateBox>,
    other_boxes: OtherBoxes,
}

impl AVCSampleEntry {
    /// Creates the entry from the code that names it, the visual fields, the
    /// configuration, and the bit rate when one is stated
    #[must_use]
    pub const fn new(
        entry_type: AVCSampleEntryType,
        visual: VisualSampleEntry,
        config: AVCConfigurationBox,
        bit_rate: Option<BitRateBox>,
    ) -> Self {
        Self {
            entry_type,
            visual,
            config,
            bit_rate,
            other_boxes: OtherBoxes::new(),
        }
    }

    /// Returns the code the entry is named by
    #[must_use]
    pub const fn entry_type(&self) -> AVCSampleEntryType {
        self.entry_type
    }

    /// Returns the fields the entry opens with
    #[must_use]
    pub const fn visual(&self) -> &VisualSampleEntry {
        &self.visual
    }

    /// Returns the configuration box, `avcC`
    #[must_use]
    pub const fn config(&self) -> &AVCConfigurationBox {
        &self.config
    }

    /// Returns the bit rate box, `btrt`, when the entry holds one
    #[must_use]
    pub const fn bit_rate(&self) -> Option<&BitRateBox> {
        self.bit_rate.as_ref()
    }

    /// Returns the boxes no field claims, in the order they came
    #[must_use]
    pub fn other_boxes(&self) -> &[AnyBox] {
        self.other_boxes.as_slice()
    }

    /// Reads the entry from the payload of a box named `entry_type`
    ///
    /// # Errors
    ///
    /// * What [`VisualSampleEntry::decode_fields`] reports for the fields.
    /// * The failures of [`boxes`]: a child does not frame as a box.
    /// * [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox): no
    ///   `avcC` follows the fields.
    /// * [`DuplicateBox`](isobmff_core::ErrorKind::DuplicateBox): more than one
    ///   `avcC` or `btrt` does.
    /// * What [`AVCConfigurationBox`] or [`BitRateBox`] reports, with its box
    ///   type on the [`containers`](Error::containers) path of the failure.
    pub fn decode_payload(entry_type: AVCSampleEntryType, payload: &[u8]) -> Result<Self, Error> {
        let mut reader = FieldReader::new(payload);
        let visual = VisualSampleEntry::decode_fields(&mut reader)?;

        let mut configuration_boxes = ChildBoxes::new();
        let mut bit_rate_boxes = ChildBoxes::new();
        let mut other_boxes = OtherBoxes::new();
        for child in boxes(reader.take_remainder()) {
            let child = child?;
            let box_type = child.header().box_type();
            if box_type == AVCConfigurationBox::BOX_TYPE {
                configuration_boxes.push(child);
            } else if box_type == BitRateBox::BOX_TYPE {
                bit_rate_boxes.push(child);
            } else {
                other_boxes.keep(child);
            }
        }

        Ok(Self {
            entry_type,
            visual,
            config: configuration_boxes.exactly_one()?,
            bit_rate: bit_rate_boxes.zero_or_one()?,
            other_boxes,
        })
    }
}

impl BoxFormat for AVCSampleEntry {
    fn box_type(&self) -> BoxType {
        self.entry_type.box_type()
    }
}

impl BoxEncode for AVCSampleEntry {
    fn payload_len(&self) -> u64 {
        let bit_rate = self.bit_rate.as_ref().map_or(0, BoxEncode::encoded_len);
        let others = self
            .other_boxes
            .as_slice()
            .iter()
            .fold(0_u64, |total, other| {
                total.saturating_add(other.encoded_len())
            });

        VisualSampleEntry::LEN
            .saturating_add(self.config.encoded_len())
            .saturating_add(bit_rate)
            .saturating_add(others)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        self.visual.encode_fields(writer)?;
        let mut rest = self.config.encode(writer.take_remainder())?;
        if let Some(bit_rate) = &self.bit_rate {
            rest = bit_rate.encode(rest)?;
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

    use isobmff_boxes::{BitRateBox, VisualSampleEntry};
    use isobmff_core::{AnyBox, BoxEncode, BoxType, Error};

    use super::{AVCSampleEntry, AVCSampleEntryType};
    use crate::avcc::tests::baseline_configuration;

    fn entry(entry_type: AVCSampleEntryType) -> AVCSampleEntry {
        AVCSampleEntry::new(
            entry_type,
            VisualSampleEntry::new(1, 1280, 720),
            baseline_configuration(),
            Some(BitRateBox::new(0, 4_000_000, 2_500_000)),
        )
    }

    fn encoded_payload(entry: &AVCSampleEntry) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(entry.payload_len()).unwrap()];
        entry.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn an_entry_reads_back_as_the_value_that_wrote_it_under_either_code() {
        for entry_type in [AVCSampleEntryType::Avc1, AVCSampleEntryType::Avc3] {
            let entry = entry(entry_type);

            let payload = encoded_payload(&entry);

            assert_eq!(
                AVCSampleEntry::decode_payload(entry_type, &payload).unwrap(),
                entry
            );
        }
    }

    #[test]
    fn an_entry_is_written_under_the_code_that_names_it() {
        let entry = entry(AVCSampleEntryType::Avc3);
        let mut buffer = vec![0; usize::try_from(entry.encoded_len()).unwrap()];

        entry.encode(&mut buffer).unwrap();

        assert_eq!(buffer.get(4..8), Some(b"avc3".as_slice()));
        assert_eq!(AnyBox::from(entry).box_type(), BoxType::compact(*b"avc3"));
    }

    #[test]
    fn the_code_is_told_from_the_box_type_of_a_stsd_entry() {
        assert_eq!(
            AVCSampleEntryType::from_box_type(BoxType::compact(*b"avc1")),
            Some(AVCSampleEntryType::Avc1)
        );
        assert_eq!(
            AVCSampleEntryType::from_box_type(BoxType::compact(*b"hvc1")),
            None
        );
    }

    #[test]
    fn a_child_no_field_claims_is_kept_and_written_back() {
        let payload = [
            encoded_payload(&entry(AVCSampleEntryType::Avc1)),
            vec![
                0, 0, 0, 0x10, b'p', b'a', b's', b'p', 0, 0, 0, 1, 0, 0, 0, 1,
            ],
        ]
        .concat();

        let entry = AVCSampleEntry::decode_payload(AVCSampleEntryType::Avc1, &payload).unwrap();

        assert_eq!(
            entry.other_boxes().first().map(AnyBox::box_type),
            Some(BoxType::compact(*b"pasp"))
        );
        assert_eq!(encoded_payload(&entry), payload);
    }

    #[test]
    fn an_entry_without_a_bit_rate_box_reads_without_one() {
        let entry = AVCSampleEntry::new(
            AVCSampleEntryType::Avc1,
            VisualSampleEntry::new(1, 16, 16),
            baseline_configuration(),
            None,
        );

        let read_back =
            AVCSampleEntry::decode_payload(AVCSampleEntryType::Avc1, &encoded_payload(&entry))
                .unwrap();

        assert_eq!(read_back, entry);
        assert_eq!(read_back.bit_rate(), None);
    }

    #[test]
    fn an_entry_holding_no_configuration_is_rejected() {
        let mut payload = vec![0; usize::try_from(VisualSampleEntry::LEN).unwrap()];
        payload.extend_from_slice(b"\0\0\0\x08free");

        assert_eq!(
            AVCSampleEntry::decode_payload(AVCSampleEntryType::Avc1, &payload),
            Err(Error::missing_mandatory_box(BoxType::compact(*b"avcC")))
        );
    }

    #[test]
    fn an_entry_holding_two_configurations_is_rejected() {
        let entry = entry(AVCSampleEntryType::Avc1);
        let mut buffer = vec![0; usize::try_from(entry.config().encoded_len()).unwrap()];
        entry.config().encode(&mut buffer).unwrap();
        let payload = [encoded_payload(&entry), buffer].concat();

        assert_eq!(
            AVCSampleEntry::decode_payload(AVCSampleEntryType::Avc1, &payload),
            Err(Error::duplicate_box(BoxType::compact(*b"avcC")))
        );
    }
}
