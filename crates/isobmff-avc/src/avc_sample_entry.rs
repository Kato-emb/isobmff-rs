//! [`AVCSampleEntry`] (`avc1`, `avc3`), ISO/IEC 14496-15 §5.4.2

use core::marker::PhantomData;

use isobmff_boxes::{BitRateBox, VisualSampleEntry};
use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes, boxes,
};

use crate::avcc::AVCConfigurationBox;

mod sealed {
    /// Bound that closes [`AVCCodingName`](super::AVCCodingName) to the codes
    /// ISO/IEC 14496-15 §5.4.2 names
    #[allow(
        unnameable_types,
        reason = "sealing AVCCodingName takes a supertrait a caller cannot name"
    )]
    pub trait Sealed {}
}

/// Coding an AVC sample entry is named by, stating where its parameter sets lie
///
/// ISO/IEC 14496-15 §5.4.2 lays one class over two codes: under [`Avc1`] the
/// parameter sets lie in the sample entry alone, under [`Avc3`] they may also
/// lie in the samples, as NAL units of the stream. This is the `codingname` of
/// ISO/IEC 14496-12 §12.1.3, which names the box a sample entry is written as.
///
/// The trait is sealed: the clause states the two codes, and no other type
/// joins them.
pub trait AVCCodingName: sealed::Sealed {
    /// Box type an entry of this coding is written under
    const BOX_TYPE: BoxType;
}

/// Coding whose parameter sets lie in the sample entry alone, `avc1`
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct Avc1;

/// Coding whose parameter sets may also lie in the samples, `avc3`
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct Avc3;

impl sealed::Sealed for Avc1 {}

impl sealed::Sealed for Avc3 {}

impl AVCCodingName for Avc1 {
    const BOX_TYPE: BoxType = BoxType::compact(*b"avc1");
}

impl AVCCodingName for Avc3 {
    const BOX_TYPE: BoxType = BoxType::compact(*b"avc3");
}

/// Sample entry of an AVC video track
///
/// [`AVCSampleEntry`] (`avc1` or `avc3`), ISO/IEC 14496-15 §5.4.2. The coding
/// the entry is named by is the type parameter, [`Avc1`] or [`Avc3`]. The entry
/// opens with the fields of a [`VisualSampleEntry`] and holds an
/// [`AVCConfigurationBox`]; a [`BitRateBox`] may follow, and any other box —
/// `m4ds`, `pasp`, `colr`, the boxes a later specification adds — is kept as it
/// came and written back.
///
/// # Examples
///
/// ```
/// use isobmff_avc::{
///     AVCConfigurationBox, AVCDecoderConfigurationRecord, Avc3SampleEntry, LengthSizeMinusOne,
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
/// let entry = Avc3SampleEntry::new(
///     VisualSampleEntry::new(1, 1920, 1080),
///     AVCConfigurationBox::new(record),
///     None,
/// );
///
/// // The entry goes into a `stsd` as any other, and comes back out typed
/// let description = SampleDescriptionBox::new(vec![AnyBox::from(entry.clone())]);
/// let found = description.entries()[0].downcast_ref::<Avc3SampleEntry>().unwrap();
/// assert_eq!(found, &entry);
/// ```
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct AVCSampleEntry<Name: AVCCodingName> {
    visual: VisualSampleEntry,
    config: AVCConfigurationBox,
    bit_rate: Option<BitRateBox>,
    other_boxes: OtherBoxes,
    _marker: PhantomData<Name>,
}

/// [`AVCSampleEntry`] named by [`Avc1`]
#[doc(alias = "avc1")]
pub type Avc1SampleEntry = AVCSampleEntry<Avc1>;

/// [`AVCSampleEntry`] named by [`Avc3`]
#[doc(alias = "avc3")]
pub type Avc3SampleEntry = AVCSampleEntry<Avc3>;

impl<Name: AVCCodingName> AVCSampleEntry<Name> {
    /// Creates the entry from the visual fields, the configuration, and the bit
    /// rate when one is stated
    #[must_use]
    pub const fn new(
        visual: VisualSampleEntry,
        config: AVCConfigurationBox,
        bit_rate: Option<BitRateBox>,
    ) -> Self {
        Self {
            visual,
            config,
            bit_rate,
            other_boxes: OtherBoxes::new(),
            _marker: PhantomData,
        }
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
}

impl<Name: AVCCodingName> BoxDefinition for AVCSampleEntry<Name> {
    const BOX_TYPE: BoxType = Name::BOX_TYPE;
}

impl<Name: AVCCodingName> BoxDecode for AVCSampleEntry<Name> {
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
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let visual = VisualSampleEntry::decode_fields(reader)?;

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
            visual,
            config: configuration_boxes.exactly_one()?,
            bit_rate: bit_rate_boxes.zero_or_one()?,
            other_boxes,
            _marker: PhantomData,
        })
    }
}

impl<Name: AVCCodingName> BoxEncode for AVCSampleEntry<Name> {
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
    use isobmff_core::{AnyBox, BoxDecode, BoxEncode, BoxType, Error};
    use isobmff_test_support::written;

    use super::{AVCCodingName, AVCSampleEntry, Avc1, Avc3};
    use crate::avcc::tests::baseline_configuration;

    fn entry<Name: AVCCodingName>() -> AVCSampleEntry<Name> {
        AVCSampleEntry::new(
            VisualSampleEntry::new(1, 1280, 720),
            baseline_configuration(),
            Some(BitRateBox::new(0, 4_000_000, 2_500_000)),
        )
    }

    fn encoded_payload<Name: AVCCodingName>(entry: &AVCSampleEntry<Name>) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(entry.payload_len()).unwrap()];
        entry.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn an_entry_reads_back_as_the_value_that_wrote_it_under_either_code() {
        let avc1 = entry::<Avc1>();
        let avc3 = entry::<Avc3>();

        assert_eq!(
            AVCSampleEntry::<Avc1>::decode_payload(&encoded_payload(&avc1)).unwrap(),
            avc1
        );
        assert_eq!(
            AVCSampleEntry::<Avc3>::decode_payload(&encoded_payload(&avc3)).unwrap(),
            avc3
        );
    }

    #[test]
    fn an_entry_is_written_under_the_code_that_names_it() {
        assert_eq!(
            written(&entry::<Avc1>()).get(4..8),
            Some(b"avc1".as_slice())
        );
        assert_eq!(
            written(&entry::<Avc3>()).get(4..8),
            Some(b"avc3".as_slice())
        );
    }

    #[test]
    fn a_child_no_field_claims_is_kept_and_written_back() {
        let payload = [
            encoded_payload(&entry::<Avc1>()),
            vec![
                0, 0, 0, 0x10, b'p', b'a', b's', b'p', 0, 0, 0, 1, 0, 0, 0, 1,
            ],
        ]
        .concat();

        let entry = AVCSampleEntry::<Avc1>::decode_payload(&payload).unwrap();

        assert_eq!(
            entry.other_boxes().first().map(AnyBox::box_type),
            Some(BoxType::compact(*b"pasp"))
        );
        assert_eq!(encoded_payload(&entry), payload);
    }

    #[test]
    fn an_entry_without_a_bit_rate_box_reads_without_one() {
        let entry = AVCSampleEntry::<Avc1>::new(
            VisualSampleEntry::new(1, 16, 16),
            baseline_configuration(),
            None,
        );

        let read_back = AVCSampleEntry::<Avc1>::decode_payload(&encoded_payload(&entry)).unwrap();

        assert_eq!(read_back, entry);
        assert_eq!(read_back.bit_rate(), None);
    }

    #[test]
    fn an_entry_holding_no_configuration_is_rejected() {
        let mut payload = vec![0; usize::try_from(VisualSampleEntry::LEN).unwrap()];
        payload.extend_from_slice(b"\0\0\0\x08free");

        assert_eq!(
            AVCSampleEntry::<Avc1>::decode_payload(&payload),
            Err(Error::missing_mandatory_box(BoxType::compact(*b"avcC")))
        );
    }

    #[test]
    fn an_entry_holding_two_configurations_is_rejected() {
        let entry = entry::<Avc1>();
        let payload = [encoded_payload(&entry), written(entry.config())].concat();

        assert_eq!(
            AVCSampleEntry::<Avc1>::decode_payload(&payload),
            Err(Error::duplicate_box(BoxType::compact(*b"avcC")))
        );
    }
}
