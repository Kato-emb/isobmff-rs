//! [`VisualSampleEntry`], the sample entry of a video track

use alloc::vec::Vec;

use isobmff_core::{
    BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FourCc, OtherBoxes, U16F16,
};

use crate::amve::AmbientViewingEnvironmentBox;
use crate::btrt::BitRateBox;
use crate::cclv::ContentColourVolumeBox;
use crate::clap::CleanApertureBox;
use crate::clli::ContentLightLevelBox;
use crate::colr::ColourInformationBox;
use crate::data_types::CompressorName;
use crate::mdcv::MasteringDisplayColourVolumeBox;
use crate::pasp::PixelAspectRatioBox;
use crate::sample_entry::{
    AnyEntryBoxes, SampleEntry, children_len, encode_children, encode_optional, from_class,
    split_children,
};
use crate::scrb::ScrambleSchemeInfoBox;
use crate::sinf::ProtectionSchemeInfoBox;

/// Length of the fields the class lays out after `data_reference_index`
const FIELDS_LEN: u64 = 16 + 2 + 2 + 4 + 4 + 4 + 2 + 32 + 2 + 2;

/// `horizresolution` and `vertresolution` of 72 dpi, the template value
const RESOLUTION_72_DPI: U16F16 = U16F16::from_integer(72);

/// [`VisualSampleEntry`], ISO/IEC 14496-12 §12.1.3
///
/// The sample entry of a video track, a [`SampleEntry`] whose bytes after
/// `data_reference_index` are read with [`TryFrom`] and written back with
/// [`SampleEntry::try_from`].
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
///    deriving the class defines, such as a decoder configuration.
/// 2. The boxes ISO/IEC 14496-12 defines for the class, each in a field of
///    its own: `clap` and `pasp` in the order the syntax of the class gives
///    them; `colr`, `clli`, `mdcv`, `cclv` and `amve`, which §12.1.5 to
///    §12.1.9 let the class hold, in that order; then the boxes any sample
///    entry may hold: `sinf`, `scrb`, then `btrt`.
/// 3. [`extra_boxes`](Self::extra_boxes): the boxes no definition of the
///    class names, which §8.5.2.1 lets any sample entry hold after the boxes
///    its definition names.
///
/// A box of a type this crate does not read cannot be told to be of a
/// derived specification or an extra box but by where it lies, so a child
/// that lies before every box of the second part is read into the first, and
/// one that lies after one of them into the third. A child of the second part
/// that does not read, such as a second `pasp`, refuses the entry.
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct VisualSampleEntry {
    /// Coding name of the samples, the `format` of the [`SampleEntry`]
    pub format: FourCc,
    /// Index from 1 of the [`DataEntry`](crate::DataEntry) the samples are retrieved from
    pub data_reference_index: u16,
    /// Largest width of the pictures the codec delivers, in pixels
    pub width: u16,
    /// Largest height of the pictures the codec delivers, in pixels
    pub height: u16,
    /// Horizontal resolution of the pictures in pixels per inch, 72 by default
    pub horiz_resolution: U16F16,
    /// Vertical resolution of the pictures in pixels per inch, 72 by default
    pub vert_resolution: U16F16,
    /// Number of frames of compressed video in each sample, 1 by default
    ///
    /// The value is 1 unless the specification of the format permits more.
    pub frame_count: u16,
    /// Name of the compressor, for informative purposes
    pub compressor_name: CompressorName,
    /// Depth of the pictures, `0x0018` by default: colour with no alpha
    pub depth: u16,
    /// Children a specification deriving the class defines, which lay before the boxes of ISO/IEC 14496-12
    pub derived_boxes: OtherBoxes,
    /// Clean aperture of the pictures
    pub clap: Option<CleanApertureBox>,
    /// Aspect ratio of a pixel
    pub pasp: Option<PixelAspectRatioBox>,
    /// Colour information, the most accurate first, in the order they came
    pub colr: Vec<ColourInformationBox>,
    /// Light level of the content
    pub clli: Option<ContentLightLevelBox>,
    /// Colour volume of the mastering display
    pub mdcv: Option<MasteringDisplayColourVolumeBox>,
    /// Colour volume of the pictures
    pub cclv: Option<ContentColourVolumeBox>,
    /// Ambient viewing environment the video was mastered for
    pub amve: Option<AmbientViewingEnvironmentBox>,
    /// Protection schemes of a protected sample entry, alternative descriptions of the same protection
    pub sinf: Vec<ProtectionSchemeInfoBox>,
    /// Scrambling schemes applied to the samples
    pub scrb: Vec<ScrambleSchemeInfoBox>,
    /// Bit rates and decoding buffer size of the stream
    pub btrt: Option<BitRateBox>,
    /// Children no definition of the class names, which lay after a box of ISO/IEC 14496-12
    pub extra_boxes: OtherBoxes,
}

impl Default for VisualSampleEntry {
    fn default() -> Self {
        Self {
            format: FourCc::new([0; 4]),
            data_reference_index: 0,
            width: 0,
            height: 0,
            horiz_resolution: RESOLUTION_72_DPI,
            vert_resolution: RESOLUTION_72_DPI,
            frame_count: 1,
            compressor_name: CompressorName::default(),
            depth: 0x0018,
            derived_boxes: OtherBoxes::new(),
            clap: None,
            pasp: None,
            colr: Vec::new(),
            clli: None,
            mdcv: None,
            cclv: None,
            amve: None,
            sinf: Vec::new(),
            scrb: Vec::new(),
            btrt: None,
            extra_boxes: OtherBoxes::new(),
        }
    }
}

/// Box types of the boxes ISO/IEC 14496-12 defines for the class
const DEFINED: [BoxType; 7] = [
    CleanApertureBox::BOX_TYPE,
    PixelAspectRatioBox::BOX_TYPE,
    ColourInformationBox::BOX_TYPE,
    ContentLightLevelBox::BOX_TYPE,
    MasteringDisplayColourVolumeBox::BOX_TYPE,
    ContentColourVolumeBox::BOX_TYPE,
    AmbientViewingEnvironmentBox::BOX_TYPE,
];

impl TryFrom<&SampleEntry> for VisualSampleEntry {
    type Error = Error;

    /// Reads the entry from the bytes of `entry` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   bytes end inside a field, counted from the end of
    ///   `data_reference_index`.
    /// * The failures of a child that does not frame, and of a box of
    ///   ISO/IEC 14496-12 given more than once.
    fn try_from(entry: &SampleEntry) -> Result<Self, Error> {
        let mut reader = FieldReader::new(&entry.remaining_data);
        reader.read_bytes::<16>()?;
        let width = reader.read_u16()?;
        let height = reader.read_u16()?;
        let horiz_resolution = U16F16::from_raw(reader.read_u32()?);
        let vert_resolution = U16F16::from_raw(reader.read_u32()?);
        reader.read_bytes::<4>()?;
        let frame_count = reader.read_u16()?;
        let compressor_name = CompressorName::from_bytes(*reader.read_bytes::<32>()?);
        let depth = reader.read_u16()?;
        reader.read_bytes::<2>()?;
        let (derived_boxes, mut children) = split_children(reader.take_remainder(), &DEFINED)?;

        Ok(Self {
            format: entry.format,
            data_reference_index: entry.data_reference_index,
            width,
            height,
            horiz_resolution,
            vert_resolution,
            frame_count,
            compressor_name,
            depth,
            derived_boxes,
            clap: children.take_zero_or_one_if_decoded()?,
            pasp: children.take_zero_or_one_if_decoded()?,
            colr: children.take_zero_or_more_if_decoded(),
            clli: children.take_zero_or_one_if_decoded()?,
            mdcv: children.take_zero_or_one_if_decoded()?,
            cclv: children.take_zero_or_one_if_decoded()?,
            amve: children.take_zero_or_one_if_decoded()?,
            sinf: children.take_zero_or_more_if_decoded(),
            scrb: children.take_zero_or_more_if_decoded(),
            btrt: children.take_zero_or_one_if_decoded()?,
            extra_boxes: OtherBoxes::from(children),
        })
    }
}

impl TryFrom<&VisualSampleEntry> for SampleEntry {
    type Error = Error;

    /// Writes `visual` into the bytes of an entry after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * The failures of writing a child.
    fn try_from(visual: &VisualSampleEntry) -> Result<Self, Error> {
        from_class(
            visual.format,
            visual.data_reference_index,
            visual.remaining_len(),
            |writer| visual.encode_remaining(writer),
        )
    }
}

impl VisualSampleEntry {
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
            self.clap.as_ref().map_or(0, BoxEncode::encoded_len),
            self.pasp.as_ref().map_or(0, BoxEncode::encoded_len),
            self.clli.as_ref().map_or(0, BoxEncode::encoded_len),
            self.mdcv.as_ref().map_or(0, BoxEncode::encoded_len),
            self.cclv.as_ref().map_or(0, BoxEncode::encoded_len),
            self.amve.as_ref().map_or(0, BoxEncode::encoded_len),
            self.any_entry_boxes().encoded_len(),
        ]
        .into_iter()
        .chain(self.colr.iter().map(BoxEncode::encoded_len));

        children_len(&self.derived_boxes, defined, &self.extra_boxes).saturating_add(FIELDS_LEN)
    }

    /// Writes the bytes the entry lays out after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * The failures of writing a child.
    fn encode_remaining(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&[0; 16])?;
        writer.write_u16(self.width)?;
        writer.write_u16(self.height)?;
        writer.write_u32(self.horiz_resolution.raw())?;
        writer.write_u32(self.vert_resolution.raw())?;
        writer.write_bytes(&[0; 4])?;
        writer.write_u16(self.frame_count)?;
        writer.write_bytes(&self.compressor_name.to_bytes())?;
        writer.write_u16(self.depth)?;
        writer.write_i16(-1)?;
        encode_children(
            writer.take_remainder(),
            &self.derived_boxes,
            |mut rest| {
                rest = encode_optional(rest, self.clap.as_ref())?;
                rest = encode_optional(rest, self.pasp.as_ref())?;
                for colr in &self.colr {
                    rest = colr.encode(rest)?;
                }
                rest = encode_optional(rest, self.clli.as_ref())?;
                rest = encode_optional(rest, self.mdcv.as_ref())?;
                rest = encode_optional(rest, self.cclv.as_ref())?;
                rest = encode_optional(rest, self.amve.as_ref())?;
                self.any_entry_boxes().encode(rest)
            },
            &self.extra_boxes,
        )
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxType, ChildBoxes, Error, FourCc, OtherBoxes};

    use super::VisualSampleEntry;
    use crate::btrt::BitRateBox;
    use crate::colr::ColourInformationBox;
    use crate::data_types::CompressorName;
    use crate::frma::OriginalFormatBox;
    use crate::pasp::PixelAspectRatioBox;
    use crate::sample_entry::SampleEntry;
    use crate::schm::SchemeTypeBox;
    use crate::sinf::ProtectionSchemeInfoBox;

    /// Fields of an entry of 176 by 144 at 72 dpi, of one frame per sample, the compressor `AVC Coding` and depth `0x0018`
    const FIELDS: &[u8] =
        b"\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\xb0\0\x90\0\x48\0\0\0\x48\0\0\0\0\0\0\0\x01\
        \x0aAVC Coding\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x18\xff\xff";

    /// Decoder configuration of a derived specification, of a type this crate does not read
    const AVCC: &[u8] = b"\0\0\0\x0bavcC\x01\x42\xc0";

    /// `pasp` of square pixels
    const PASP: &[u8] = b"\0\0\0\x10pasp\0\0\0\x01\0\0\0\x01";

    /// `colr` of the on-screen colours of BT.709 in full range
    const COLR: &[u8] = b"\0\0\0\x13colrnclx\0\x01\0\x01\0\x01\x80";

    /// `btrt` of a buffer of 4096 bytes, at most 2 Mbit/s and 1 Mbit/s on average
    const BTRT: &[u8] = b"\0\0\0\x14btrt\0\0\x10\0\0\x1e\x84\x80\0\x0f\x42\x40";

    /// `sinf` of the original format `avc1` and the scheme `cenc` version 1.0
    const SINF: &[u8] = b"\0\0\0\x28sinf\0\0\0\x0cfrmaavc1\0\0\0\x14schm\0\0\0\0cenc\0\x01\0\0";

    /// Box no definition of the class names
    const EXTRA: &[u8] = b"\0\0\0\x09xtra\x07";

    fn sample_entry(children: &[&[u8]]) -> SampleEntry {
        SampleEntry {
            format: FourCc::new(*b"avc1"),
            data_reference_index: 1,
            remaining_data: [&[FIELDS], children].concat().concat(),
        }
    }

    fn kept(children: &[&[u8]]) -> OtherBoxes {
        OtherBoxes::from(ChildBoxes::decode(&children.concat()).unwrap())
    }

    fn visual(derived_boxes: &[&[u8]], extra_boxes: &[&[u8]]) -> VisualSampleEntry {
        VisualSampleEntry {
            format: FourCc::new(*b"avc1"),
            data_reference_index: 1,
            width: 176,
            height: 144,
            compressor_name: CompressorName::new("AVC Coding").unwrap(),
            derived_boxes: kept(derived_boxes),
            pasp: Some(PixelAspectRatioBox {
                h_spacing: 1,
                v_spacing: 1,
            }),
            colr: vec![ColourInformationBox::OnScreenColours {
                colour_primaries: 1,
                transfer_characteristics: 1,
                matrix_coefficients: 1,
                full_range_flag: true,
            }],
            btrt: Some(BitRateBox {
                buffer_size_db: 4096,
                max_bitrate: 2_000_000,
                avg_bitrate: 1_000_000,
            }),
            extra_boxes: kept(extra_boxes),
            ..VisualSampleEntry::default()
        }
    }

    #[test]
    fn reads_the_boxes_before_those_of_the_class_as_derived_and_those_after_as_extra() {
        assert_eq!(
            VisualSampleEntry::try_from(&sample_entry(&[AVCC, PASP, COLR, EXTRA, BTRT])),
            Ok(visual(&[AVCC], &[EXTRA]))
        );
    }

    #[test]
    fn writes_the_derived_boxes_then_those_of_the_class_in_its_order_then_the_extra_boxes() {
        let read =
            VisualSampleEntry::try_from(&sample_entry(&[AVCC, BTRT, COLR, EXTRA, PASP])).unwrap();

        assert_eq!(
            SampleEntry::try_from(&read),
            Ok(sample_entry(&[AVCC, PASP, COLR, BTRT, EXTRA]))
        );
    }

    fn protected(children: &[&[u8]]) -> SampleEntry {
        SampleEntry {
            format: FourCc::new(*b"encv"),
            ..sample_entry(children)
        }
    }

    fn protected_visual() -> VisualSampleEntry {
        VisualSampleEntry {
            format: FourCc::new(*b"encv"),
            sinf: vec![ProtectionSchemeInfoBox {
                frma: Some(OriginalFormatBox {
                    data_format: FourCc::new(*b"avc1"),
                }),
                schm: Some(SchemeTypeBox {
                    scheme_type: FourCc::new(*b"cenc"),
                    scheme_version: 0x0001_0000,
                    scheme_uri: None,
                }),
                ..ProtectionSchemeInfoBox::default()
            }],
            ..visual(&[AVCC], &[])
        }
    }

    #[test]
    fn a_protected_entry_reads_with_the_syntax_of_its_original_class() {
        assert_eq!(
            VisualSampleEntry::try_from(&protected(&[AVCC, SINF, PASP, COLR, BTRT])),
            Ok(protected_visual())
        );
    }

    #[test]
    fn writes_the_sinf_of_a_protected_entry_after_the_boxes_of_the_class_and_before_btrt() {
        assert_eq!(
            SampleEntry::try_from(&protected_visual()),
            Ok(protected(&[AVCC, PASP, COLR, SINF, BTRT]))
        );
    }

    #[test]
    fn an_entry_of_no_box_of_the_class_holds_every_child_as_derived() {
        let expected = VisualSampleEntry {
            pasp: None,
            colr: Vec::new(),
            btrt: None,
            ..visual(&[AVCC, EXTRA], &[])
        };

        assert_eq!(
            VisualSampleEntry::try_from(&sample_entry(&[AVCC, EXTRA])),
            Ok(expected)
        );
    }

    #[test]
    fn bytes_ending_inside_a_field_are_refused() {
        let entry = SampleEntry {
            remaining_data: FIELDS.get(..69).unwrap().to_vec(),
            ..sample_entry(&[])
        };

        assert_eq!(
            VisualSampleEntry::try_from(&entry),
            Err(Error::truncated_payload(70, 69))
        );
    }

    #[test]
    fn a_box_of_the_class_given_twice_is_refused() {
        assert_eq!(
            VisualSampleEntry::try_from(&sample_entry(&[PASP, PASP])).map(|_| ()),
            Err(Error::duplicate_box(BoxType::compact(*b"pasp")))
        );
    }
}
