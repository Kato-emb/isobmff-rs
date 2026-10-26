//! [`VolumetricVisualSampleEntry`], the sample entry of a volumetric visual track

use alloc::vec::Vec;

use isobmff_core::{Error, FieldReader, FourCc, OtherBoxes};

use crate::btrt::BitRateBox;
use crate::data_types::CompressorName;
use crate::sample_entry::{
    AnyEntryBoxes, AnyEntryChildren, SampleEntry, children_len, encode_children, from_class,
};
use crate::scrb::ScrambleSchemeInfoBox;
use crate::sinf::ProtectionSchemeInfoBox;

/// [`VolumetricVisualSampleEntry`], ISO/IEC 14496-12 §12.9.3
///
/// The sample entry of a volumetric visual track, whose samples the coding
/// system lays out. The bytes of a [`SampleEntry`] after
/// `data_reference_index` are read with [`TryFrom`] and written back with
/// [`SampleEntry::try_from`].
///
/// # Children
///
/// The children are laid out in three parts as those of a
/// [`FontSampleEntry`](crate::FontSampleEntry) are, around the boxes any
/// sample entry may hold.
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct VolumetricVisualSampleEntry {
    /// Coding name of the samples, the `format` of the [`SampleEntry`]
    pub format: FourCc,
    /// Index from 1 of the [`DataEntry`](crate::DataEntry) the samples are retrieved from
    pub data_reference_index: u16,
    /// Name of the compressor, for informative purposes
    pub compressor_name: CompressorName,
    /// Children a specification deriving the class defines, which lay before the boxes of ISO/IEC 14496-12
    pub derived_boxes: OtherBoxes,
    /// Protection schemes of a protected sample entry, alternative descriptions of the same protection
    pub sinf: Vec<ProtectionSchemeInfoBox>,
    /// Scrambling schemes applied to the samples
    pub scrb: Vec<ScrambleSchemeInfoBox>,
    /// Bit rates and decoding buffer size of the stream
    pub btrt: Option<BitRateBox>,
    /// Children no definition of the class names, which lay after a box of ISO/IEC 14496-12
    pub extra_boxes: OtherBoxes,
}

impl Default for VolumetricVisualSampleEntry {
    fn default() -> Self {
        Self {
            format: FourCc::new([0; 4]),
            data_reference_index: 0,
            compressor_name: CompressorName::default(),
            derived_boxes: OtherBoxes::new(),
            sinf: Vec::new(),
            scrb: Vec::new(),
            btrt: None,
            extra_boxes: OtherBoxes::new(),
        }
    }
}

impl TryFrom<&SampleEntry> for VolumetricVisualSampleEntry {
    type Error = Error;

    /// Reads the entry from the bytes of `entry` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   bytes end inside `compressorname`.
    /// * The failures of a child that does not frame, and of a `btrt` given
    ///   more than once.
    fn try_from(entry: &SampleEntry) -> Result<Self, Error> {
        let mut reader = FieldReader::new(&entry.remaining_data);
        let compressor_name = CompressorName::from_bytes(*reader.read_bytes::<32>()?);
        let children = AnyEntryChildren::read(reader.take_remainder())?;

        Ok(Self {
            format: entry.format,
            data_reference_index: entry.data_reference_index,
            compressor_name,
            derived_boxes: children.derived_boxes,
            sinf: children.sinf,
            scrb: children.scrb,
            btrt: children.btrt,
            extra_boxes: children.extra_boxes,
        })
    }
}

impl TryFrom<&VolumetricVisualSampleEntry> for SampleEntry {
    type Error = Error;

    /// Writes `volumetric` into the bytes of an entry after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * The failures of writing a child.
    fn try_from(volumetric: &VolumetricVisualSampleEntry) -> Result<Self, Error> {
        let any_entry = AnyEntryBoxes {
            sinf: &volumetric.sinf,
            scrb: &volumetric.scrb,
            btrt: volumetric.btrt.as_ref(),
        };

        from_class(
            volumetric.format,
            volumetric.data_reference_index,
            children_len(
                &volumetric.derived_boxes,
                [any_entry.encoded_len()],
                &volumetric.extra_boxes,
            )
            .saturating_add(32),
            |writer| {
                writer.write_bytes(&volumetric.compressor_name.to_bytes())?;
                encode_children(
                    writer.take_remainder(),
                    &volumetric.derived_boxes,
                    |rest| any_entry.encode(rest),
                    &volumetric.extra_boxes,
                )
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{ChildBoxes, Error, FourCc, OtherBoxes};

    use super::VolumetricVisualSampleEntry;
    use crate::btrt::BitRateBox;
    use crate::data_types::CompressorName;
    use crate::sample_entry::SampleEntry;

    /// `compressorname` of `V-PCC`
    const COMPRESSOR_NAME: &[u8] = b"\x05V-PCC\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";

    /// Decoder configuration of a derived specification, of a type this crate does not read
    const CONFIGURATION: &[u8] = b"\0\0\0\x09vpcC\x01";

    /// `btrt` of a buffer of 4096 bytes, at most 2 Mbit/s and 1 Mbit/s on average
    const BTRT: &[u8] = b"\0\0\0\x14btrt\0\0\x10\0\0\x1e\x84\x80\0\x0f\x42\x40";

    fn sample_entry(remaining: &[&[u8]]) -> SampleEntry {
        SampleEntry {
            format: FourCc::new(*b"v3c1"),
            data_reference_index: 1,
            remaining_data: remaining.concat(),
        }
    }

    fn volumetric(extra_boxes: &[&[u8]]) -> VolumetricVisualSampleEntry {
        VolumetricVisualSampleEntry {
            format: FourCc::new(*b"v3c1"),
            data_reference_index: 1,
            compressor_name: CompressorName::new("V-PCC").unwrap(),
            derived_boxes: OtherBoxes::from(ChildBoxes::decode(CONFIGURATION).unwrap()),
            btrt: Some(BitRateBox {
                buffer_size_db: 4096,
                max_bitrate: 2_000_000,
                avg_bitrate: 1_000_000,
            }),
            extra_boxes: OtherBoxes::from(ChildBoxes::decode(&extra_boxes.concat()).unwrap()),
            ..VolumetricVisualSampleEntry::default()
        }
    }

    #[test]
    fn reads_its_compressor_name_then_its_boxes_split_by_where_they_lie() {
        assert_eq!(
            VolumetricVisualSampleEntry::try_from(&sample_entry(&[
                COMPRESSOR_NAME,
                CONFIGURATION,
                BTRT,
                CONFIGURATION
            ])),
            Ok(volumetric(&[CONFIGURATION]))
        );
    }

    #[test]
    fn writes_its_compressor_name_then_its_boxes_in_the_order_of_its_class() {
        assert_eq!(
            SampleEntry::try_from(&volumetric(&[])),
            Ok(sample_entry(&[COMPRESSOR_NAME, CONFIGURATION, BTRT]))
        );
    }

    #[test]
    fn bytes_ending_inside_the_compressor_name_are_refused() {
        assert_eq!(
            VolumetricVisualSampleEntry::try_from(&sample_entry(&[COMPRESSOR_NAME
                .get(..31)
                .unwrap()])),
            Err(Error::truncated_payload(32, 31))
        );
    }
}
