//! [`AudioSampleEntry`], the sample entry of an audio track, and [`AudioSampleEntryVariant`], which reads either class of audio sample entry

use alloc::vec::Vec;

use isobmff_core::{
    BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader, FieldWriter, FourCc,
    OtherBoxes, U16F16,
};

use crate::btrt::BitRateBox;
use crate::chnl::{ChannelLayout, ChannelStructure};
use crate::dmix::{DownMixInstructions, UndecodedDownMixInstructions};
use crate::sample_entry::audio_v1::AudioSampleEntryV1;
use crate::sample_entry::{
    AnyEntryBoxes, SampleEntry, children_len, encode_children, encode_optional, from_class,
    split_children,
};
use crate::scrb::ScrambleSchemeInfoBox;
use crate::sinf::ProtectionSchemeInfoBox;
use crate::srat::SamplingRateBox;

/// Box types of the boxes ISO/IEC 14496-12 defines for both classes
const DEFINED: [BoxType; 3] = [
    ChannelLayout::BOX_TYPE,
    DownMixInstructions::BOX_TYPE,
    SamplingRateBox::BOX_TYPE,
];

/// Returns the `baseChannelCount` of an entry of `channel_count` channels holding `chnl`
///
/// §12.2.5.3 derives it from the `chnl`, where the `chnl` gives it: the
/// `base_channel_count` of version 1, or the positions of an explicit layout
/// of version 0 and its audio objects. Otherwise it is `channel_count`.
fn base_channel_count(chnl: Option<&ChannelLayout>, channel_count: u16) -> u16 {
    match chnl {
        Some(ChannelLayout::Version1 {
            base_channel_count, ..
        }) => u16::from(*base_channel_count),
        Some(ChannelLayout::Version0 {
            channel_structure: Some(ChannelStructure::Explicit { speaker_positions }),
            object_count,
        }) => u16::try_from(speaker_positions.len())
            .unwrap_or(u16::MAX)
            .saturating_add(u16::from(object_count.unwrap_or(0))),
        _ => channel_count,
    }
}

/// Checks that each of `dmix`, of an entry of `channel_count` channels holding `chnl`, holds the coefficients of the `baseChannelCount` the entry reads it with
///
/// # Errors
///
/// * [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
///   `baseChannelCount`: a `dmix` holds the coefficients of another count,
///   which would not read back.
pub(super) fn check_downmixes(
    chnl: Option<&ChannelLayout>,
    channel_count: u16,
    dmix: &[DownMixInstructions],
) -> Result<(), Error> {
    let base_channel_count = usize::from(base_channel_count(chnl, channel_count));
    for instructions in dmix {
        if instructions
            .base_channel_count()?
            .is_some_and(|held| held != base_channel_count)
        {
            return Err(Error::invalid_value("baseChannelCount"));
        }
    }

    Ok(())
}

/// Takes the `chnl` and the `dmix` boxes out of `children` of an entry of `channel_count` channels, the `dmix` boxes read with the `baseChannelCount` the `chnl` gives
///
/// # Errors
///
/// * The failures of [`DownMixInstructions`] read with the
///   `baseChannelCount`, and of a box given more than once.
pub(super) fn take_channel_boxes(
    children: &mut ChildBoxes<'_>,
    channel_count: u16,
) -> Result<(Option<ChannelLayout>, Vec<DownMixInstructions>), Error> {
    let chnl = children.take_zero_or_one_if_decoded()?;
    let base_channel_count = base_channel_count(chnl.as_ref(), channel_count);
    let dmix = children
        .take_zero_or_more_if_decoded::<UndecodedDownMixInstructions>()
        .into_iter()
        .map(|undecoded| DownMixInstructions::decode_payload(base_channel_count, &undecoded.0))
        .collect::<Result<_, _>>()?;

    Ok((chnl, dmix))
}

/// Fields both classes lay out after `data_reference_index`, from `channelcount` on
pub(super) struct Fields {
    pub(super) channel_count: u16,
    pub(super) sample_size: u16,
    pub(super) sample_rate: U16F16,
}

impl Fields {
    /// Length of the fields both classes lay out after `data_reference_index`, `entry_version` included
    pub(super) const ENCODED_LEN: u64 = 2 + 6 + 2 + 2 + 2 + 2 + 4;

    /// Reads the fields of an entry of the class of `entry_version` from the bytes of `entry` after `data_reference_index`, and returns them with the children that follow, split as [`split_children`] splits them
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   bytes end inside a field.
    /// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
    ///   `entry_version`: the entry is of the other class.
    /// * The failures of [`split_children`].
    pub(super) fn read(
        entry: &SampleEntry,
        entry_version: u16,
    ) -> Result<(Self, OtherBoxes, ChildBoxes<'_>), Error> {
        let mut reader = FieldReader::new(&entry.remaining_data);
        if reader.read_u16()? != entry_version {
            return Err(Error::unsupported_value("entry_version"));
        }
        reader.read_bytes::<6>()?;
        let channel_count = reader.read_u16()?;
        let sample_size = reader.read_u16()?;
        reader.read_bytes::<4>()?;
        let sample_rate = U16F16::from_raw(reader.read_u32()?);
        let (derived_boxes, children) = split_children(reader.take_remainder(), &DEFINED)?;

        Ok((
            Self {
                channel_count,
                sample_size,
                sample_rate,
            },
            derived_boxes,
            children,
        ))
    }

    /// Writes the fields of an entry of the class of `entry_version`
    ///
    /// # Errors
    ///
    /// * [`TruncatedBuffer`](isobmff_core::ErrorKind::TruncatedBuffer): the
    ///   buffer ends inside the fields.
    pub(super) fn write(
        &self,
        writer: &mut FieldWriter<'_>,
        entry_version: u16,
    ) -> Result<(), Error> {
        writer.write_u16(entry_version)?;
        writer.write_bytes(&[0; 6])?;
        writer.write_u16(self.channel_count)?;
        writer.write_u16(self.sample_size)?;
        writer.write_bytes(&[0; 4])?;
        writer.write_u32(self.sample_rate.raw())
    }
}

/// [`AudioSampleEntry`], ISO/IEC 14496-12 §12.2.3
///
/// The sample entry of an audio track that §12.2.3.1 prefers for maximum
/// compatibility, a [`SampleEntry`] whose bytes after `data_reference_index`
/// open with an `entry_version` of 0, the first of its `reserved` bits. Those
/// bytes are read with [`TryFrom`] and written back with
/// [`SampleEntry::try_from`]; [`AudioSampleEntryVariant`] reads an entry of
/// either class.
///
/// The `pre_defined` and `reserved` fields are read past, and written as the
/// specification states them.
///
/// # Children
///
/// The children are laid out in three parts, the order in which the entry
/// writes them, whatever order they were read in:
///
/// 1. [`derived_boxes`](Self::derived_boxes): the boxes that a specification
///    deriving the class defines, such as a decoder configuration, and the
///    DRC boxes of ISO/IEC 23003-4.
/// 2. The boxes ISO/IEC 14496-12 defines for the class, each in a field of
///    its own: `chnl`, `dmix` and `srat` in the order the syntax of the
///    class gives them, then the boxes any sample entry may hold: `sinf`,
///    `scrb`, then `btrt`.
/// 3. [`extra_boxes`](Self::extra_boxes): the boxes no definition of the
///    class names.
///
/// A box of a type this crate does not read cannot be told to be of a
/// derived specification or an extra box but by where it lies, so a child
/// that lies before every box of the second part is read into the first, and
/// one that lies after one of them into the third. A child of the second part
/// that does not read, such as a second `srat`, refuses the entry.
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct AudioSampleEntry {
    /// Coding name of the samples, the `format` of the [`SampleEntry`]
    pub format: FourCc,
    /// Index from 1 of the [`DataEntry`](crate::DataEntry) the samples are retrieved from
    pub data_reference_index: u16,
    /// Number of channels: 0 inapplicable or unknown, 1 mono, 2 stereo, and otherwise as the codec configuration assigns them
    ///
    /// The field is no template, so [`Default`] gives 0, unknown.
    pub channel_count: u16,
    /// Size of a sample in bits, 16 by default
    pub sample_size: u16,
    /// Sampling rate of the audio, or with an `srat` a multiple or division of it, 0 by default
    ///
    /// The template value is the default sampling rate of the media, which a
    /// track sets.
    pub sample_rate: U16F16,
    /// Children a specification deriving the class defines, which lay before the boxes of ISO/IEC 14496-12
    pub derived_boxes: OtherBoxes,
    /// Assignment of the channels
    pub chnl: Option<ChannelLayout>,
    /// Downmixes, read with the `baseChannelCount` that `chnl` gives, or else `channel_count`
    pub dmix: Vec<DownMixInstructions>,
    /// Actual sampling rate of the audio, which `sample_rate` cannot carry
    pub srat: Option<SamplingRateBox>,
    /// Protection schemes of a protected sample entry, alternative descriptions of the same protection
    pub sinf: Vec<ProtectionSchemeInfoBox>,
    /// Scrambling schemes applied to the samples
    pub scrb: Vec<ScrambleSchemeInfoBox>,
    /// Bit rates and decoding buffer size of the stream
    pub btrt: Option<BitRateBox>,
    /// Children no definition of the class names, which lay after a box of ISO/IEC 14496-12
    pub extra_boxes: OtherBoxes,
}

impl Default for AudioSampleEntry {
    fn default() -> Self {
        Self {
            format: FourCc::new([0; 4]),
            data_reference_index: 0,
            channel_count: 0,
            sample_size: 16,
            sample_rate: U16F16::from_raw(0),
            derived_boxes: OtherBoxes::new(),
            chnl: None,
            dmix: Vec::new(),
            srat: None,
            sinf: Vec::new(),
            scrb: Vec::new(),
            btrt: None,
            extra_boxes: OtherBoxes::new(),
        }
    }
}

impl TryFrom<&SampleEntry> for AudioSampleEntry {
    type Error = Error;

    /// Reads the entry from the bytes of `entry` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   bytes end inside a field, counted from the end of
    ///   `data_reference_index`.
    /// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
    ///   `entry_version`: the bytes open with an `entry_version` other than
    ///   0.
    /// * The failures of a child that does not frame, of a box of
    ///   ISO/IEC 14496-12 given more than once, and of a `dmix` read with the
    ///   `baseChannelCount` the entry gives.
    fn try_from(entry: &SampleEntry) -> Result<Self, Error> {
        let (fields, derived_boxes, mut children) = Fields::read(entry, 0)?;
        let (chnl, dmix) = take_channel_boxes(&mut children, fields.channel_count)?;

        Ok(Self {
            format: entry.format,
            data_reference_index: entry.data_reference_index,
            channel_count: fields.channel_count,
            sample_size: fields.sample_size,
            sample_rate: fields.sample_rate,
            derived_boxes,
            chnl,
            dmix,
            srat: children.take_zero_or_one_if_decoded()?,
            sinf: children.take_zero_or_more_if_decoded(),
            scrb: children.take_zero_or_more_if_decoded(),
            btrt: children.take_zero_or_one_if_decoded()?,
            extra_boxes: OtherBoxes::from(children),
        })
    }
}

impl TryFrom<&AudioSampleEntry> for SampleEntry {
    type Error = Error;

    /// Writes `audio` into the bytes of an entry after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * The failures of writing a child.
    fn try_from(audio: &AudioSampleEntry) -> Result<Self, Error> {
        from_class(
            audio.format,
            audio.data_reference_index,
            audio.remaining_len(),
            |writer| audio.encode_remaining(writer),
        )
    }
}

impl AudioSampleEntry {
    /// Returns the boxes of the entry that any sample entry may hold
    fn any_entry_boxes(&self) -> AnyEntryBoxes<'_> {
        AnyEntryBoxes {
            sinf: &self.sinf,
            scrb: &self.scrb,
            btrt: self.btrt.as_ref(),
        }
    }

    /// Returns the length of the bytes the entry lays out after `data_reference_index`
    fn remaining_len(&self) -> u64 {
        let defined = [
            self.chnl.as_ref().map_or(0, BoxEncode::encoded_len),
            self.srat.as_ref().map_or(0, BoxEncode::encoded_len),
            self.any_entry_boxes().encoded_len(),
        ]
        .into_iter()
        .chain(self.dmix.iter().map(BoxEncode::encoded_len));

        children_len(&self.derived_boxes, defined, &self.extra_boxes)
            .saturating_add(Fields::ENCODED_LEN)
    }

    /// Writes the bytes the entry lays out after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
    ///   `baseChannelCount`: a `dmix` holds the coefficients of another
    ///   `baseChannelCount` than the entry gives.
    /// * The failures of writing a child.
    fn encode_remaining(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        check_downmixes(self.chnl.as_ref(), self.channel_count, &self.dmix)?;
        let fields = Fields {
            channel_count: self.channel_count,
            sample_size: self.sample_size,
            sample_rate: self.sample_rate,
        };
        fields.write(writer, 0)?;
        encode_children(
            writer.take_remainder(),
            &self.derived_boxes,
            |mut rest| {
                rest = encode_optional(rest, self.chnl.as_ref())?;
                for dmix in &self.dmix {
                    rest = dmix.encode(rest)?;
                }
                rest = encode_optional(rest, self.srat.as_ref())?;
                self.any_entry_boxes().encode(rest)
            },
            &self.extra_boxes,
        )
    }
}

/// Sample entry of an audio track, of either class §12.2.3 lets an audio track use
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub enum AudioSampleEntryVariant {
    /// Entry of the `entry_version` 0
    AudioSampleEntry(AudioSampleEntry),
    /// Entry of the `entry_version` 1
    AudioSampleEntryV1(AudioSampleEntryV1),
}

impl TryFrom<&SampleEntry> for AudioSampleEntryVariant {
    type Error = Error;

    /// Reads the entry into the class its `entry_version`, the first 16 bits after `data_reference_index`, names
    ///
    /// # Errors
    ///
    /// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
    ///   `entry_version`: the `entry_version` is neither 0 nor 1.
    /// * The failures of reading the entry into its class.
    fn try_from(entry: &SampleEntry) -> Result<Self, Error> {
        match FieldReader::new(&entry.remaining_data).read_u16()? {
            1 => AudioSampleEntryV1::try_from(entry).map(Self::AudioSampleEntryV1),
            _ => AudioSampleEntry::try_from(entry).map(Self::AudioSampleEntry),
        }
    }
}

impl TryFrom<&AudioSampleEntryVariant> for SampleEntry {
    type Error = Error;

    /// Writes the entry of either class into the bytes of an entry after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * The failures of writing a child.
    fn try_from(audio: &AudioSampleEntryVariant) -> Result<Self, Error> {
        match audio {
            AudioSampleEntryVariant::AudioSampleEntry(audio) => Self::try_from(audio),
            AudioSampleEntryVariant::AudioSampleEntryV1(audio) => Self::try_from(audio),
        }
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{ChildBoxes, Error, FourCc, OtherBoxes, U16F16};

    use alloc::vec;

    use super::{AudioSampleEntry, AudioSampleEntryVariant};
    use crate::btrt::BitRateBox;
    use crate::chnl::ChannelLayout;
    use crate::dmix::{DownMixInstruction, DownMixInstructions, DownmixCoefficients};
    use crate::sample_entry::SampleEntry;
    use crate::sample_entry::audio_v1::AudioSampleEntryV1;
    use crate::srat::SamplingRateBox;

    /// Fields of an entry of `entry_version` 0: 2 channels of 16 bits at 48000 samples per second
    const FIELDS: &[u8] = b"\0\0\0\0\0\0\0\0\0\x02\0\x10\0\0\0\0\xbb\x80\0\0";

    /// Fields of an entry of `entry_version` 1: 2 channels of 16 bits, of a sample rate of 1
    const FIELDS_V1: &[u8] = b"\0\x01\0\0\0\0\0\0\0\x02\0\x10\0\0\0\0\0\x01\0\0";

    /// Decoder configuration of a derived specification, of a type this crate does not read
    const ESDS: &[u8] = b"\0\0\0\x0desds\0\0\0\0\x03";

    /// `chnl` of version 1 of 3 base channels and no structure
    const CHNL: &[u8] = b"\0\0\0\x0echnl\x01\0\0\0\0\x03";

    /// `dmix` of version 1 of one downmix 1 from 3 base channels to the layout 1 of 1 channel, each coefficient 0 dB
    const DMIX: &[u8] = b"\0\0\0\x13dmix\x01\0\0\0\x01\x01\x01\x01\x02\x94\xa0";

    /// `dmix` of the downmix of [`DMIX`] from 2 base channels
    const DMIX_OF_2: &[u8] = b"\0\0\0\x12dmix\x01\0\0\0\x01\x01\x01\x01\x02\x94";

    /// `srat` of 96000 samples per second
    const SRAT: &[u8] = b"\0\0\0\x10srat\0\0\0\0\0\x01\x77\0";

    /// `btrt` of a buffer of 4096 bytes, at most 2 Mbit/s and 1 Mbit/s on average
    const BTRT: &[u8] = b"\0\0\0\x14btrt\0\0\x10\0\0\x1e\x84\x80\0\x0f\x42\x40";

    /// Box no definition of the class names
    const EXTRA: &[u8] = b"\0\0\0\x09xtra\x07";

    fn sample_entry(fields: &[u8], children: &[&[u8]]) -> SampleEntry {
        SampleEntry {
            format: FourCc::new(*b"mp4a"),
            data_reference_index: 1,
            remaining_data: [&[fields], children].concat().concat(),
        }
    }

    fn kept(children: &[&[u8]]) -> OtherBoxes {
        OtherBoxes::from(ChildBoxes::decode(&children.concat()).unwrap())
    }

    fn btrt() -> Option<BitRateBox> {
        Some(BitRateBox {
            buffer_size_db: 4096,
            max_bitrate: 2_000_000,
            avg_bitrate: 1_000_000,
        })
    }

    fn srat() -> Option<SamplingRateBox> {
        Some(SamplingRateBox {
            sampling_rate: 96_000,
        })
    }

    fn chnl() -> Option<ChannelLayout> {
        Some(ChannelLayout::Version1 {
            format_ordering: 0,
            base_channel_count: 3,
            channel_structure: None,
            object_structured: false,
        })
    }

    fn dmix(base_channel_count: usize) -> DownMixInstructions {
        DownMixInstructions {
            version: 1,
            downmix_instructions: vec![DownMixInstruction {
                target_layout: 1,
                target_channel_count: 1,
                downmix_id: 1,
                downmix_coefficients: Some(DownmixCoefficients::Version1 {
                    bs_downmix_offset: 0,
                    bs_downmix_coefficient_v1: vec![5; base_channel_count],
                }),
            }],
        }
    }

    fn audio() -> AudioSampleEntry {
        AudioSampleEntry {
            format: FourCc::new(*b"mp4a"),
            data_reference_index: 1,
            channel_count: 2,
            sample_rate: U16F16::from_integer(48_000),
            derived_boxes: kept(&[ESDS]),
            chnl: chnl(),
            dmix: vec![dmix(3)],
            srat: srat(),
            btrt: btrt(),
            extra_boxes: kept(&[EXTRA]),
            ..AudioSampleEntry::default()
        }
    }

    fn audio_v1() -> AudioSampleEntryV1 {
        AudioSampleEntryV1 {
            format: FourCc::new(*b"mp4a"),
            data_reference_index: 1,
            channel_count: 2,
            derived_boxes: kept(&[ESDS]),
            chnl: chnl(),
            dmix: vec![dmix(3)],
            srat: srat(),
            btrt: btrt(),
            extra_boxes: kept(&[EXTRA]),
            ..AudioSampleEntryV1::default()
        }
    }

    #[test]
    fn reads_the_class_of_its_entry_version() {
        assert_eq!(
            [
                AudioSampleEntryVariant::try_from(&sample_entry(
                    FIELDS,
                    &[ESDS, DMIX, SRAT, EXTRA, BTRT, CHNL]
                )),
                AudioSampleEntryVariant::try_from(&sample_entry(
                    FIELDS_V1,
                    &[ESDS, BTRT, DMIX, EXTRA, CHNL, SRAT]
                )),
            ],
            [
                Ok(AudioSampleEntryVariant::AudioSampleEntry(audio())),
                Ok(AudioSampleEntryVariant::AudioSampleEntryV1(audio_v1())),
            ]
        );
    }

    #[test]
    fn writes_the_derived_boxes_then_those_of_the_class_in_its_order_then_the_extra_boxes() {
        assert_eq!(
            [
                SampleEntry::try_from(&AudioSampleEntryVariant::AudioSampleEntry(audio())),
                SampleEntry::try_from(&AudioSampleEntryVariant::AudioSampleEntryV1(audio_v1())),
            ],
            [
                Ok(sample_entry(FIELDS, &[ESDS, CHNL, DMIX, SRAT, BTRT, EXTRA])),
                Ok(sample_entry(
                    FIELDS_V1,
                    &[ESDS, SRAT, CHNL, DMIX, BTRT, EXTRA]
                )),
            ]
        );
    }

    #[test]
    fn downmixes_of_an_entry_without_a_channel_layout_have_its_channel_count_for_base() {
        let expected = AudioSampleEntry {
            format: FourCc::new(*b"mp4a"),
            data_reference_index: 1,
            channel_count: 2,
            sample_rate: U16F16::from_integer(48_000),
            dmix: vec![dmix(2)],
            ..AudioSampleEntry::default()
        };

        assert_eq!(
            AudioSampleEntry::try_from(&sample_entry(FIELDS, &[DMIX_OF_2])),
            Ok(expected)
        );
    }

    #[test]
    fn a_downmix_that_does_not_read_with_the_base_channel_count_refuses_the_entry() {
        assert_eq!(
            AudioSampleEntry::try_from(&sample_entry(FIELDS, &[CHNL, DMIX_OF_2])),
            Err(Error::truncated_payload(11, 10))
        );
    }

    #[test]
    fn downmixes_of_another_base_channel_count_than_the_entry_gives_are_refused_when_written() {
        let audio = AudioSampleEntry {
            channel_count: 3,
            dmix: vec![dmix(1)],
            ..AudioSampleEntry::default()
        };
        let audio_v1 = AudioSampleEntryV1 {
            chnl: chnl(),
            dmix: vec![dmix(2)],
            ..AudioSampleEntryV1::default()
        };

        assert_eq!(
            (
                SampleEntry::try_from(&audio),
                SampleEntry::try_from(&audio_v1)
            ),
            (
                Err(Error::invalid_value("baseChannelCount")),
                Err(Error::invalid_value("baseChannelCount"))
            )
        );
    }

    #[test]
    fn an_entry_is_refused_by_the_class_of_the_other_entry_version() {
        assert_eq!(
            (
                AudioSampleEntry::try_from(&sample_entry(FIELDS_V1, &[])),
                AudioSampleEntryV1::try_from(&sample_entry(FIELDS, &[])),
            ),
            (
                Err(Error::unsupported_value("entry_version")),
                Err(Error::unsupported_value("entry_version"))
            )
        );
    }

    #[test]
    fn an_entry_version_other_than_0_or_1_is_refused() {
        let mut fields = FIELDS.to_vec();
        *fields.get_mut(1).unwrap() = 2;

        assert_eq!(
            AudioSampleEntryVariant::try_from(&sample_entry(&fields, &[])),
            Err(Error::unsupported_value("entry_version"))
        );
    }

    #[test]
    fn bytes_ending_inside_a_field_are_refused() {
        assert_eq!(
            AudioSampleEntry::try_from(&sample_entry(FIELDS.get(..19).unwrap(), &[])),
            Err(Error::truncated_payload(20, 19))
        );
    }
}
