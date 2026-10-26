//! [`AudioSampleEntryV1`], the sample entry of an audio track for a sampling rate only an `srat` carries

use alloc::vec::Vec;

use isobmff_core::{BoxEncode, Error, FieldWriter, FourCc, OtherBoxes, U16F16};

use crate::btrt::BitRateBox;
use crate::chnl::ChannelLayout;
use crate::dmix::DownMixInstructions;
use crate::sample_entry::audio::{Fields, take_channel_boxes};
use crate::sample_entry::{
    SampleEntry, children_len, encode_children, encode_optional, from_class,
};
use crate::srat::SamplingRateBox;

/// [`AudioSampleEntryV1`], ISO/IEC 14496-12 §12.2.3
///
/// The sample entry of an audio track for a sampling rate that only an `srat`
/// can carry, a [`SampleEntry`] whose bytes after `data_reference_index` open
/// with an `entry_version` of 1. It lies in a `stsd` of version 1, which is not
/// checked. The bytes are read with [`TryFrom`] and written back with
/// [`SampleEntry::try_from`]; [`AudioSampleEntryVariant`](crate::AudioSampleEntryVariant) reads an entry of
/// either class.
///
/// The `pre_defined` and `reserved` fields are read past, and written as the
/// specification states them.
///
/// # Children
///
/// The children are laid out in three parts as those of an
/// [`AudioSampleEntry`](crate::AudioSampleEntry) are, the boxes of ISO/IEC 14496-12 in the order the
/// syntax of the class gives them, `srat`, `chnl` and `dmix`, then `btrt`.
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct AudioSampleEntryV1 {
    /// Coding name of the samples, the `format` of the [`SampleEntry`]
    pub format: FourCc,
    /// Index from 1 of the [`DataEntry`](crate::DataEntry) the samples are retrieved from
    pub data_reference_index: u16,
    /// Number of channels, which shall be correct: 1 mono, 2 stereo, and otherwise as the codec configuration assigns them
    ///
    /// The template gives no value, so [`Default`] gives 0, unknown.
    pub channel_count: u16,
    /// Size of a sample in bits, 16 by default
    pub sample_size: u16,
    /// Sampling rate of the audio as a multiple or division of the `srat`, 1 by default
    pub sample_rate: U16F16,
    /// Children a specification deriving the class defines, which lay before the boxes of ISO/IEC 14496-12
    pub derived_boxes: OtherBoxes,
    /// Actual sampling rate of the audio, which overrides `sample_rate`
    pub srat: Option<SamplingRateBox>,
    /// Assignment of the channels
    pub chnl: Option<ChannelLayout>,
    /// Downmixes, read with the `baseChannelCount` that `chnl` gives, or else `channel_count`
    pub dmix: Vec<DownMixInstructions>,
    /// Bit rates and decoding buffer size of the stream
    pub btrt: Option<BitRateBox>,
    /// Children no definition of the class names, which lay after a box of ISO/IEC 14496-12
    pub extra_boxes: OtherBoxes,
}

impl Default for AudioSampleEntryV1 {
    fn default() -> Self {
        Self {
            format: FourCc::new([0; 4]),
            data_reference_index: 0,
            channel_count: 0,
            sample_size: 16,
            sample_rate: U16F16::from_integer(1),
            derived_boxes: OtherBoxes::new(),
            chnl: None,
            dmix: Vec::new(),
            srat: None,
            btrt: None,
            extra_boxes: OtherBoxes::new(),
        }
    }
}

impl TryFrom<&SampleEntry> for AudioSampleEntryV1 {
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
    ///   1.
    /// * The failures of a child that does not frame, of a box of
    ///   ISO/IEC 14496-12 given more than once, and of a `dmix` read with the
    ///   `baseChannelCount` the entry gives.
    fn try_from(entry: &SampleEntry) -> Result<Self, Error> {
        let (fields, derived_boxes, mut children) = Fields::read(entry, 1)?;
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
            btrt: children.take_zero_or_one_if_decoded()?,
            extra_boxes: OtherBoxes::from(children),
        })
    }
}

impl TryFrom<&AudioSampleEntryV1> for SampleEntry {
    type Error = Error;

    /// Writes `audio` into the bytes of an entry after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * The failures of writing a child.
    fn try_from(audio: &AudioSampleEntryV1) -> Result<Self, Error> {
        from_class(
            audio.format,
            audio.data_reference_index,
            audio.remaining_len(),
            |writer| audio.encode_remaining(writer),
        )
    }
}

impl AudioSampleEntryV1 {
    /// Returns the length of the bytes the entry lays out after `data_reference_index`
    fn remaining_len(&self) -> u64 {
        let defined = [
            self.chnl.as_ref().map_or(0, BoxEncode::encoded_len),
            self.srat.as_ref().map_or(0, BoxEncode::encoded_len),
            self.btrt.as_ref().map_or(0, BoxEncode::encoded_len),
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
    /// * The failures of writing a child.
    fn encode_remaining(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let fields = Fields {
            channel_count: self.channel_count,
            sample_size: self.sample_size,
            sample_rate: self.sample_rate,
        };
        fields.write(writer, 1)?;
        encode_children(
            writer.take_remainder(),
            &self.derived_boxes,
            |mut rest| {
                rest = encode_optional(rest, self.srat.as_ref())?;
                rest = encode_optional(rest, self.chnl.as_ref())?;
                for dmix in &self.dmix {
                    rest = dmix.encode(rest)?;
                }
                encode_optional(rest, self.btrt.as_ref())
            },
            &self.extra_boxes,
        )
    }
}
